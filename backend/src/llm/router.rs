//! Dynamic multi-model routing.
//!
//! `TaskKind → tier → ordered model aliases`. The router adds three behaviours on top of
//! the static table:
//!
//! * **escalation** — after repeated failures (TDD loop), climb the capability ladder;
//! * **budget awareness** — when the run's budget is nearly spent, step down the ladder,
//!   and refuse calls once it is exhausted;
//! * **fallback** — if a model errors out, try the next alias of the tier.
//!
//! Aliases (`omniforge-reasoning`, …) are resolved to concrete provider models by the
//! LiteLLM proxy (`config/litellm.config.yaml`): swapping vendors never touches this code.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{LlmBackend, LlmRequest, LlmResponse, TaskKind};
use crate::error::{Error, Result};
use crate::events::{Emitter, EventType};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TierConfig {
    #[serde(default)]
    pub description: String,
    /// Model aliases, preferred first; the rest are fallbacks.
    pub models: Vec<String>,
}

/// USD per million tokens. Only used when the gateway does not report the call cost.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Price {
    pub input: f64,
    pub output: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RoutingConfig {
    pub tiers: BTreeMap<String, TierConfig>,
    /// Task kind (snake_case) → tier name.
    pub routes: BTreeMap<String, String>,
    /// Capability ladder, weakest first. Escalation climbs it, budget pressure descends it.
    pub escalation: Vec<String>,
    /// Below this fraction of remaining budget, calls are routed one step down the ladder.
    pub low_budget_ratio: f64,
    pub pricing: BTreeMap<String, Price>,
}

impl Default for RoutingConfig {
    fn default() -> Self {
        let tier = |description: &str, models: &[&str]| TierConfig {
            description: description.to_string(),
            models: models.iter().map(|m| m.to_string()).collect(),
        };
        let tiers = BTreeMap::from([
            (
                "reasoning".to_string(),
                tier(
                    "Raisonnement long : planification, architecture, code complexe",
                    &["omniforge-reasoning", "omniforge-coder"],
                ),
            ),
            (
                "coder".to_string(),
                tier("Implémentation courante, bon rapport qualité/coût", &["omniforge-coder", "omniforge-reasoning"]),
            ),
            (
                "critic".to_string(),
                tier(
                    "Tests et critique par un fournisseur différent du codeur (erreurs décorrélées)",
                    &["omniforge-critic", "omniforge-coder"],
                ),
            ),
            (
                "routine".to_string(),
                tier(
                    "Tâches de routine à très faible coût : lint, parsing, classification, résumés",
                    &["omniforge-local", "omniforge-fast"],
                ),
            ),
        ]);
        let routes = [
            (TaskKind::Planning, "reasoning"),
            (TaskKind::Architecture, "reasoning"),
            (TaskKind::ComplexCode, "reasoning"),
            (TaskKind::Code, "coder"),
            (TaskKind::TestGeneration, "critic"),
            (TaskKind::Critique, "critic"),
            (TaskKind::SecurityReview, "critic"),
            (TaskKind::LintFix, "routine"),
            (TaskKind::Classification, "routine"),
            (TaskKind::Summarization, "routine"),
        ]
        .into_iter()
        .map(|(k, t)| (k.as_str().to_string(), t.to_string()))
        .collect();
        let pricing = BTreeMap::from([
            (
                "omniforge-reasoning".to_string(),
                Price {
                    input: 4.0,
                    output: 20.0,
                },
            ),
            (
                "omniforge-coder".to_string(),
                Price {
                    input: 2.0,
                    output: 10.0,
                },
            ),
            (
                "omniforge-critic".to_string(),
                Price {
                    input: 2.0,
                    output: 10.0,
                },
            ),
            (
                "omniforge-fast".to_string(),
                Price {
                    input: 0.1,
                    output: 0.5,
                },
            ),
            (
                "omniforge-local".to_string(),
                Price {
                    input: 0.0,
                    output: 0.0,
                },
            ),
        ]);
        Self {
            tiers,
            routes,
            escalation: vec!["routine".into(), "coder".into(), "reasoning".into()],
            low_budget_ratio: 0.15,
            pricing,
        }
    }
}

impl RoutingConfig {
    pub fn validate(&self) -> Result<()> {
        for kind in TaskKind::ALL {
            let tier = self.routes.get(kind.as_str()).ok_or_else(|| {
                Error::Invalid(format!(
                    "routing.routes has no entry for '{}'",
                    kind.as_str()
                ))
            })?;
            if !self.tiers.contains_key(tier) {
                return Err(Error::Invalid(format!(
                    "routing.routes.{} points to unknown tier '{tier}'",
                    kind.as_str()
                )));
            }
        }
        for (name, tier) in &self.tiers {
            if tier.models.is_empty() {
                return Err(Error::Invalid(format!("routing.tiers.{name} has no model")));
            }
        }
        for tier in &self.escalation {
            if !self.tiers.contains_key(tier) {
                return Err(Error::Invalid(format!(
                    "routing.escalation references unknown tier '{tier}'"
                )));
            }
        }
        Ok(())
    }
}

/// Spend tracker for one run.
#[derive(Debug)]
pub struct Budget {
    limit: Mutex<Option<f64>>,
    spent: Mutex<f64>,
}

impl Budget {
    pub fn new(limit: Option<f64>) -> Self {
        Self {
            limit: Mutex::new(limit),
            spent: Mutex::new(0.0),
        }
    }

    pub fn limit(&self) -> Option<f64> {
        *self.limit.lock().expect("budget poisoned")
    }

    pub fn set_limit(&self, limit: Option<f64>) {
        *self.limit.lock().expect("budget poisoned") = limit;
    }

    pub fn spent(&self) -> f64 {
        *self.spent.lock().expect("budget poisoned")
    }

    pub fn add(&self, cost: f64) {
        *self.spent.lock().expect("budget poisoned") += cost;
    }

    /// Fraction of the budget still available (`None` when unlimited).
    pub fn remaining_ratio(&self) -> Option<f64> {
        self.limit().map(|l| {
            if l <= 0.0 {
                0.0
            } else {
                ((l - self.spent()) / l).max(0.0)
            }
        })
    }

    pub fn exhausted(&self) -> bool {
        self.limit().is_some_and(|l| self.spent() >= l)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RouteDecision {
    pub tier: String,
    pub models: Vec<String>,
    /// Human-readable explanation, surfaced in the dashboard.
    pub reason: String,
}

pub struct ModelRouter {
    backend: Arc<dyn LlmBackend>,
    cfg: RoutingConfig,
}

impl ModelRouter {
    pub fn new(backend: Arc<dyn LlmBackend>, cfg: RoutingConfig) -> Self {
        Self { backend, cfg }
    }

    pub fn config(&self) -> &RoutingConfig {
        &self.cfg
    }

    pub fn backend_name(&self) -> &'static str {
        self.backend.name()
    }

    /// Pure routing decision (no I/O): which tier and which models, and why.
    pub fn route(
        &self,
        kind: TaskKind,
        escalation: u32,
        remaining_budget_ratio: Option<f64>,
    ) -> RouteDecision {
        let base = self
            .cfg
            .routes
            .get(kind.as_str())
            .cloned()
            .unwrap_or_else(|| "coder".to_string());
        let mut tier = base.clone();
        let mut reason = format!("{} → palier « {base} »", kind.as_str());
        let ladder = &self.cfg.escalation;

        if let Some(pos) = ladder.iter().position(|t| *t == tier) {
            let mut idx = pos;
            if escalation > 0 {
                idx = (pos + escalation as usize).min(ladder.len() - 1);
                if idx != pos {
                    reason.push_str(&format!(
                        " ; escalade ×{escalation} après échecs → « {} »",
                        ladder[idx]
                    ));
                }
            }
            if remaining_budget_ratio.is_some_and(|r| r < self.cfg.low_budget_ratio) && idx > 0 {
                idx -= 1;
                reason.push_str(&format!(
                    " ; budget presque épuisé → rétrogradation vers « {} »",
                    ladder[idx]
                ));
            }
            tier = ladder[idx].clone();
        }

        let models = self
            .cfg
            .tiers
            .get(&tier)
            .map(|t| t.models.clone())
            .unwrap_or_default();
        RouteDecision {
            tier,
            models,
            reason,
        }
    }

    fn estimate_cost(&self, model: &str, prompt_tokens: u64, completion_tokens: u64) -> f64 {
        self.cfg
            .pricing
            .get(model)
            .map(|p| {
                (prompt_tokens as f64 * p.input + completion_tokens as f64 * p.output) / 1_000_000.0
            })
            .unwrap_or(0.0)
    }

    /// Routes and executes a call, walking the fallback chain. Emits one `llm.call` event
    /// per attempt so cost and routing are fully observable.
    pub async fn complete(
        &self,
        request: LlmRequest,
        budget: &Budget,
        emitter: &Emitter,
    ) -> Result<LlmResponse> {
        if budget.exhausted() {
            return Err(Error::BudgetExceeded {
                spent: budget.spent(),
                limit: budget.limit().unwrap_or(0.0),
            });
        }
        let decision = self.route(
            request.kind,
            request.meta.escalation,
            budget.remaining_ratio(),
        );
        let source = format!("agent:{}", request.meta.role);
        let task_id = request.meta.task_id.clone();
        let mut last_error =
            Error::Llm(format!("no model configured for tier '{}'", decision.tier));

        for (index, model) in decision.models.iter().enumerate() {
            let started = Instant::now();
            match self.backend.complete(model, &request).await {
                Ok(response) => {
                    let cost = response.cost_usd.unwrap_or_else(|| {
                        self.estimate_cost(
                            model,
                            response.prompt_tokens,
                            response.completion_tokens,
                        )
                    });
                    budget.add(cost);
                    emitter.emit(
                        EventType::LlmCall,
                        &source,
                        task_id.as_deref(),
                        json!({
                            "ok": true,
                            "kind": request.kind,
                            "phase": request.meta.phase,
                            "tier": decision.tier,
                            "model": model,
                            "served_by": response.model,
                            "backend": self.backend.name(),
                            "fallback_index": index,
                            "reason": decision.reason,
                            "prompt_tokens": response.prompt_tokens,
                            "completion_tokens": response.completion_tokens,
                            "cost_usd": cost,
                            "cost_estimated": response.cost_usd.is_none(),
                            "latency_ms": started.elapsed().as_millis() as u64,
                            "budget_spent_usd": budget.spent(),
                            "budget_limit_usd": budget.limit(),
                        }),
                    );
                    return Ok(response);
                }
                Err(err) => {
                    emitter.emit(
                        EventType::LlmCall,
                        &source,
                        task_id.as_deref(),
                        json!({
                            "ok": false,
                            "kind": request.kind,
                            "phase": request.meta.phase,
                            "tier": decision.tier,
                            "model": model,
                            "backend": self.backend.name(),
                            "fallback_index": index,
                            "reason": decision.reason,
                            "error": err.to_string(),
                            "latency_ms": started.elapsed().as_millis() as u64,
                        }),
                    );
                    last_error = err;
                }
            }
        }
        Err(Error::Llm(format!(
            "all models of tier '{}' failed ({}); last error: {last_error}",
            decision.tier,
            decision.models.join(", ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventStore;
    use crate::llm::{CallMeta, ChatMessage};
    use async_trait::async_trait;

    struct Flaky {
        fail: Vec<&'static str>,
    }

    #[async_trait]
    impl LlmBackend for Flaky {
        fn name(&self) -> &'static str {
            "flaky"
        }
        async fn complete(&self, model: &str, _request: &LlmRequest) -> Result<LlmResponse> {
            if self.fail.contains(&model) {
                return Err(Error::Llm(format!("{model} is down")));
            }
            Ok(LlmResponse {
                content: Some("ok".into()),
                model: model.to_string(),
                prompt_tokens: 1_000_000,
                completion_tokens: 0,
                ..Default::default()
            })
        }
    }

    fn router(fail: Vec<&'static str>) -> ModelRouter {
        ModelRouter::new(Arc::new(Flaky { fail }), RoutingConfig::default())
    }

    fn request(kind: TaskKind, escalation: u32) -> LlmRequest {
        let meta = CallMeta {
            role: "tester".into(),
            escalation,
            ..Default::default()
        };
        LlmRequest::new(kind, vec![ChatMessage::user("hi")], meta)
    }

    #[test]
    fn routes_each_kind_to_its_tier() {
        let r = router(vec![]);
        assert_eq!(r.route(TaskKind::Architecture, 0, None).tier, "reasoning");
        assert_eq!(r.route(TaskKind::Code, 0, None).tier, "coder");
        assert_eq!(r.route(TaskKind::TestGeneration, 0, None).tier, "critic");
        assert_eq!(
            r.route(TaskKind::LintFix, 0, None).models[0],
            "omniforge-local"
        );
        RoutingConfig::default().validate().unwrap();
    }

    #[test]
    fn escalates_after_failures_and_downgrades_on_low_budget() {
        let r = router(vec![]);
        assert_eq!(r.route(TaskKind::Code, 1, None).tier, "reasoning");
        assert_eq!(r.route(TaskKind::LintFix, 1, None).tier, "coder");
        assert_eq!(r.route(TaskKind::LintFix, 9, None).tier, "reasoning");
        assert_eq!(r.route(TaskKind::Code, 0, Some(0.05)).tier, "routine");
        assert_eq!(r.route(TaskKind::Code, 0, Some(0.5)).tier, "coder");
        // Tiers outside the ladder (critic) keep their vendor diversity.
        assert_eq!(r.route(TaskKind::Critique, 3, Some(0.01)).tier, "critic");
    }

    #[tokio::test]
    async fn falls_back_to_the_next_model_and_tracks_cost() {
        let store = EventStore::open(None).unwrap();
        let emitter = Emitter::new(store.clone(), "r1");
        let budget = Budget::new(Some(10.0));
        let r = router(vec!["omniforge-coder"]);
        let res = r
            .complete(request(TaskKind::Code, 0), &budget, &emitter)
            .await
            .unwrap();
        assert_eq!(res.model, "omniforge-reasoning");
        // 1M prompt tokens on omniforge-reasoning at $4/MTok (fallback pricing table).
        assert!((budget.spent() - 4.0).abs() < 1e-9);
        let events = store.events("r1", 0);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].payload["ok"], false);
        assert_eq!(events[1].payload["fallback_index"], 1);
    }

    #[tokio::test]
    async fn refuses_calls_once_the_budget_is_exhausted() {
        let store = EventStore::open(None).unwrap();
        let emitter = Emitter::new(store, "r1");
        let budget = Budget::new(Some(1.0));
        budget.add(1.5);
        let err = router(vec![])
            .complete(request(TaskKind::Code, 0), &budget, &emitter)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::BudgetExceeded { .. }));
    }

    #[tokio::test]
    async fn reports_failure_when_every_model_fails() {
        let store = EventStore::open(None).unwrap();
        let emitter = Emitter::new(store, "r1");
        let r = router(vec!["omniforge-coder", "omniforge-reasoning"]);
        let err = r
            .complete(request(TaskKind::Code, 0), &Budget::new(None), &emitter)
            .await
            .unwrap_err();
        assert!(err
            .to_string()
            .contains("all models of tier 'coder' failed"));
    }
}
