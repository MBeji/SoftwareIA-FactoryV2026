//! Governance "à la carte": the supervision matrix (HITL / HOTL / Autonomous) and the
//! gatekeeper that turns it into approve / pause / block decisions at every checkpoint.

pub mod security;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::oneshot;

use crate::error::{Error, Result};
use crate::events::{Emitter, EventType};

/// How much human supervision a stage gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupervisionMode {
    /// Human-In-The-Loop: mandatory pause at every key checkpoint.
    Hitl,
    /// Human-On-The-Loop: autonomous, pauses only on exceptions.
    Hotl,
    /// Zero-touch: never waits for a human; blocking exceptions fail the run instead.
    Autonomous,
}

/// Checkpoints of the SDLC workflow where the matrix is evaluated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Plan,
    Architecture,
    TestPlan,
    /// Only evaluated on exception (TDD divergence).
    Implementation,
    /// Only evaluated on exception (blocking security findings).
    Security,
    Merge,
    /// Only evaluated on exception (budget exhausted).
    Budget,
}

impl Stage {
    pub const ALL: [Stage; 7] = [
        Stage::Plan,
        Stage::Architecture,
        Stage::TestPlan,
        Stage::Implementation,
        Stage::Security,
        Stage::Merge,
        Stage::Budget,
    ];

    /// Stages where HITL always pauses, even when everything looks fine.
    pub fn is_key_checkpoint(self) -> bool {
        matches!(
            self,
            Stage::Plan | Stage::Architecture | Stage::TestPlan | Stage::Merge
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

/// Per-project (and live-editable per-run) supervision settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SupervisionPolicy {
    pub default_mode: SupervisionMode,
    /// Per-stage overrides, e.g. `{ merge = "hitl" }` on an otherwise autonomous project.
    pub stage_overrides: BTreeMap<Stage, SupervisionMode>,
    /// HOTL pauses when an agent's confidence falls below this threshold (0..1).
    pub confidence_threshold: f64,
    /// Red→Green attempts allowed before the loop is declared divergent.
    pub max_tdd_iterations: u32,
    /// "Request changes" rounds a human may ask for on a single gate.
    pub max_revisions: u32,
    /// LLM spend ceiling for the run, in USD. `None` = unlimited.
    pub budget_usd: Option<f64>,
    /// Findings at or above this severity are a security violation.
    pub block_on_severity: Severity,
    /// Auto-reject a pending gate after this many seconds (fail-safe). `None` = wait forever.
    pub gate_timeout_s: Option<u64>,
}

impl Default for SupervisionPolicy {
    fn default() -> Self {
        Self {
            default_mode: SupervisionMode::Hotl,
            stage_overrides: BTreeMap::new(),
            confidence_threshold: 0.7,
            max_tdd_iterations: 4,
            max_revisions: 2,
            budget_usd: None,
            block_on_severity: Severity::High,
            gate_timeout_s: None,
        }
    }
}

impl SupervisionPolicy {
    pub fn mode_for(&self, stage: Stage) -> SupervisionMode {
        self.stage_overrides
            .get(&stage)
            .copied()
            .unwrap_or(self.default_mode)
    }

    pub fn validate(&self) -> Result<()> {
        if !(0.0..=1.0).contains(&self.confidence_threshold) {
            return Err(Error::Invalid(
                "confidence_threshold must be within [0, 1]".into(),
            ));
        }
        if !(1..=20).contains(&self.max_tdd_iterations) {
            return Err(Error::Invalid(
                "max_tdd_iterations must be within [1, 20]".into(),
            ));
        }
        if self.budget_usd.is_some_and(|b| b <= 0.0) {
            return Err(Error::Invalid("budget_usd must be positive".into()));
        }
        Ok(())
    }
}

/// Why an otherwise autonomous flow may need a human.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExceptionKind {
    LowConfidence,
    SecurityViolation,
    TddDivergence,
    BudgetExceeded,
    PolicyViolation,
    VacuousTests,
}

impl ExceptionKind {
    /// Blocking exceptions can never be waved through without a human:
    /// in autonomous mode they fail the run. Missing or vacuous tests are blocking because
    /// the TDD cycle is mandatory — code is never written against tests that prove nothing.
    pub fn is_blocking(self) -> bool {
        matches!(
            self,
            ExceptionKind::SecurityViolation
                | ExceptionKind::TddDivergence
                | ExceptionKind::BudgetExceeded
                | ExceptionKind::VacuousTests
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExceptionSignal {
    pub kind: ExceptionKind,
    pub detail: String,
}

impl ExceptionSignal {
    pub fn new(kind: ExceptionKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    AutoApproved,
    HumanRequired,
    Blocked,
}

/// Result of walking the supervision decision tree for one checkpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evaluation {
    pub stage: Stage,
    pub mode: SupervisionMode,
    pub confidence: f64,
    pub threshold: f64,
    pub exceptions: Vec<ExceptionSignal>,
    pub verdict: Verdict,
    pub reason: String,
}

/// The supervision decision tree. Pure function: easy to test, easy to explain in the UI.
pub fn evaluate(
    policy: &SupervisionPolicy,
    stage: Stage,
    confidence: f64,
    exceptions: &[ExceptionSignal],
) -> Evaluation {
    let mode = policy.mode_for(stage);
    let mut exceptions = exceptions.to_vec();
    if confidence < policy.confidence_threshold
        && !exceptions
            .iter()
            .any(|e| e.kind == ExceptionKind::LowConfidence)
    {
        exceptions.push(ExceptionSignal::new(
            ExceptionKind::LowConfidence,
            format!(
                "confiance {:.0} % inférieure au seuil {:.0} %",
                confidence * 100.0,
                policy.confidence_threshold * 100.0
            ),
        ));
    }
    let details = || {
        exceptions
            .iter()
            .map(|e| e.detail.as_str())
            .collect::<Vec<_>>()
            .join(" ; ")
    };

    let (verdict, reason) = match mode {
        SupervisionMode::Hitl if stage.is_key_checkpoint() => (
            Verdict::HumanRequired,
            "HITL : validation humaine obligatoire à cette étape clé".to_string(),
        ),
        SupervisionMode::Hitl | SupervisionMode::Hotl if !exceptions.is_empty() => (
            Verdict::HumanRequired,
            format!("supervision par exception : {}", details()),
        ),
        SupervisionMode::Autonomous if exceptions.iter().any(|e| e.kind.is_blocking()) => (
            Verdict::Blocked,
            format!(
                "mode autonome : exception bloquante sans humain disponible ({})",
                details()
            ),
        ),
        SupervisionMode::Autonomous if !exceptions.is_empty() => (
            Verdict::AutoApproved,
            format!(
                "mode autonome : exception non bloquante consignée ({})",
                details()
            ),
        ),
        _ => (
            Verdict::AutoApproved,
            "aucune exception : poursuite automatique".to_string(),
        ),
    };

    Evaluation {
        stage,
        mode,
        confidence,
        threshold: policy.confidence_threshold,
        exceptions,
        verdict,
        reason,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateDecision {
    Approve,
    RequestChanges,
    Reject,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateResolution {
    pub decision: GateDecision,
    #[serde(default)]
    pub feedback: Option<String>,
    /// Who decided (user id, "cli", "timeout"…).
    #[serde(default = "default_resolver")]
    pub resolver: String,
}

fn default_resolver() -> String {
    "user".to_string()
}

/// Label shown on a decision button; lets each gate explain what a decision means here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateOption {
    pub decision: GateDecision,
    pub label: String,
}

impl GateOption {
    pub fn new(decision: GateDecision, label: &str) -> Self {
        Self {
            decision,
            label: label.to_string(),
        }
    }

    pub fn defaults() -> Vec<GateOption> {
        vec![
            GateOption::new(GateDecision::Approve, "Approuver"),
            GateOption::new(GateDecision::RequestChanges, "Demander des modifications"),
            GateOption::new(GateDecision::Reject, "Rejeter et arrêter le run"),
        ]
    }
}

/// What the orchestrator submits at a checkpoint.
pub struct Checkpoint {
    pub stage: Stage,
    pub task_id: Option<String>,
    pub title: String,
    pub summary: String,
    /// Structured proposal rendered by the approval portal (plan, architecture, diff…).
    pub proposal: Value,
    pub confidence: f64,
    pub exceptions: Vec<ExceptionSignal>,
    pub options: Vec<GateOption>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GateOutcome {
    Approved {
        feedback: Option<String>,
        by_human: bool,
    },
    ChangesRequested {
        feedback: String,
    },
    Rejected {
        reason: String,
        by_human: bool,
    },
}

/// Evaluates checkpoints against the (live-editable) policy and parks the run on a
/// oneshot channel whenever a human decision is required.
pub struct GateKeeper {
    emitter: Emitter,
    policy: Arc<RwLock<SupervisionPolicy>>,
    pending: Mutex<HashMap<String, oneshot::Sender<GateResolution>>>,
}

impl GateKeeper {
    pub fn new(emitter: Emitter, policy: Arc<RwLock<SupervisionPolicy>>) -> Self {
        Self {
            emitter,
            policy,
            pending: Mutex::new(HashMap::new()),
        }
    }

    pub fn policy(&self) -> SupervisionPolicy {
        self.policy.read().expect("policy poisoned").clone()
    }

    pub fn pending_gate_ids(&self) -> Vec<String> {
        self.pending
            .lock()
            .expect("gates poisoned")
            .keys()
            .cloned()
            .collect()
    }

    pub async fn checkpoint(&self, cp: Checkpoint) -> GateOutcome {
        let policy = self.policy();
        let eval = evaluate(&policy, cp.stage, cp.confidence, &cp.exceptions);
        let task_id = cp.task_id.as_deref();
        self.emitter.emit(
            EventType::GateEvaluated,
            "gatekeeper",
            task_id,
            json!({ "title": cp.title, "evaluation": eval }),
        );

        match eval.verdict {
            Verdict::AutoApproved => GateOutcome::Approved {
                feedback: None,
                by_human: false,
            },
            Verdict::Blocked => GateOutcome::Rejected {
                reason: eval.reason,
                by_human: false,
            },
            Verdict::HumanRequired => {
                let gate_id = format!("gate-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
                let (tx, rx) = oneshot::channel();
                self.pending
                    .lock()
                    .expect("gates poisoned")
                    .insert(gate_id.clone(), tx);
                self.emitter.emit(
                    EventType::GateRequested,
                    "gatekeeper",
                    task_id,
                    json!({
                        "gate_id": gate_id,
                        "stage": cp.stage,
                        "mode": eval.mode,
                        "title": cp.title,
                        "summary": cp.summary,
                        "reason": eval.reason,
                        "confidence": cp.confidence,
                        "threshold": eval.threshold,
                        "exceptions": eval.exceptions,
                        "proposal": cp.proposal,
                        "options": cp.options,
                    }),
                );
                self.emitter.emit(
                    EventType::RunStatusChanged,
                    "gatekeeper",
                    None,
                    json!({ "status": "waiting_approval", "gate_id": gate_id }),
                );

                let resolution = match policy.gate_timeout_s {
                    Some(secs) => match tokio::time::timeout(Duration::from_secs(secs), rx).await {
                        Ok(res) => res.ok(),
                        Err(_) => Some(GateResolution {
                            decision: GateDecision::Reject,
                            feedback: Some(format!("aucune décision humaine après {secs} s")),
                            resolver: "timeout".into(),
                        }),
                    },
                    None => rx.await.ok(),
                }
                .unwrap_or(GateResolution {
                    decision: GateDecision::Reject,
                    feedback: Some("portail fermé avant décision".into()),
                    resolver: "system".into(),
                });
                self.pending
                    .lock()
                    .expect("gates poisoned")
                    .remove(&gate_id);

                self.emitter.emit(
                    EventType::GateResolved,
                    "gatekeeper",
                    task_id,
                    json!({
                        "gate_id": gate_id,
                        "stage": cp.stage,
                        "decision": resolution.decision,
                        "feedback": resolution.feedback,
                        "resolver": resolution.resolver,
                    }),
                );
                self.emitter.emit(
                    EventType::RunStatusChanged,
                    "gatekeeper",
                    None,
                    json!({ "status": "running" }),
                );

                // An expired or abandoned gate is the factory giving up, not a human "no":
                // the run ends as failed, not as cancelled by its supervisor.
                let by_human = !matches!(resolution.resolver.as_str(), "timeout" | "system");
                let feedback = resolution.feedback.filter(|f| !f.trim().is_empty());
                match resolution.decision {
                    GateDecision::Approve => GateOutcome::Approved { feedback, by_human },
                    GateDecision::RequestChanges => GateOutcome::ChangesRequested {
                        feedback: feedback
                            .unwrap_or_else(|| "modifications demandées (sans précision)".into()),
                    },
                    GateDecision::Reject => GateOutcome::Rejected {
                        reason: feedback.unwrap_or_else(|| "rejeté par le superviseur".into()),
                        by_human,
                    },
                }
            }
        }
    }

    /// Delivers a human decision to the parked checkpoint.
    pub fn resolve(&self, gate_id: &str, resolution: GateResolution) -> Result<()> {
        let tx = self
            .pending
            .lock()
            .expect("gates poisoned")
            .remove(gate_id)
            .ok_or_else(|| Error::NotFound(format!("no pending gate '{gate_id}'")))?;
        tx.send(resolution)
            .map_err(|_| Error::Conflict("gate is no longer waiting".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(mode: SupervisionMode) -> SupervisionPolicy {
        SupervisionPolicy {
            default_mode: mode,
            ..Default::default()
        }
    }

    #[test]
    fn hitl_pauses_on_key_checkpoints_only() {
        let p = policy(SupervisionMode::Hitl);
        for stage in Stage::ALL {
            let v = evaluate(&p, stage, 0.95, &[]).verdict;
            let expected = if stage.is_key_checkpoint() {
                Verdict::HumanRequired
            } else {
                Verdict::AutoApproved
            };
            assert_eq!(v, expected, "{stage:?}");
        }
    }

    #[test]
    fn hotl_pauses_only_on_exception() {
        let p = policy(SupervisionMode::Hotl);
        assert_eq!(
            evaluate(&p, Stage::Architecture, 0.9, &[]).verdict,
            Verdict::AutoApproved
        );
        // Low confidence is detected by the tree itself.
        let low = evaluate(&p, Stage::Architecture, 0.4, &[]);
        assert_eq!(low.verdict, Verdict::HumanRequired);
        assert_eq!(low.exceptions[0].kind, ExceptionKind::LowConfidence);
        let div = [ExceptionSignal::new(
            ExceptionKind::TddDivergence,
            "4 itérations",
        )];
        assert_eq!(
            evaluate(&p, Stage::Implementation, 0.9, &div).verdict,
            Verdict::HumanRequired
        );
    }

    #[test]
    fn autonomous_never_waits_but_blocks_on_blocking_exceptions() {
        let p = policy(SupervisionMode::Autonomous);
        assert_eq!(
            evaluate(&p, Stage::Merge, 0.9, &[]).verdict,
            Verdict::AutoApproved
        );
        assert_eq!(
            evaluate(&p, Stage::Plan, 0.2, &[]).verdict,
            Verdict::AutoApproved
        );
        let sec = [ExceptionSignal::new(
            ExceptionKind::SecurityViolation,
            "secret en dur",
        )];
        assert_eq!(
            evaluate(&p, Stage::Security, 0.9, &sec).verdict,
            Verdict::Blocked
        );
        let vacuous = [ExceptionSignal::new(
            ExceptionKind::VacuousTests,
            "aucun test écrit",
        )];
        assert_eq!(
            evaluate(&p, Stage::TestPlan, 0.9, &vacuous).verdict,
            Verdict::Blocked
        );
        let tampering = [ExceptionSignal::new(
            ExceptionKind::PolicyViolation,
            "commande refusée",
        )];
        assert_eq!(
            evaluate(&p, Stage::Merge, 0.9, &tampering).verdict,
            Verdict::AutoApproved
        );
    }

    #[test]
    fn stage_override_wins_over_default_mode() {
        let mut p = policy(SupervisionMode::Autonomous);
        p.stage_overrides
            .insert(Stage::Merge, SupervisionMode::Hitl);
        assert_eq!(
            evaluate(&p, Stage::Merge, 0.99, &[]).verdict,
            Verdict::HumanRequired
        );
        assert_eq!(
            evaluate(&p, Stage::Plan, 0.99, &[]).verdict,
            Verdict::AutoApproved
        );
    }

    fn keeper(policy: SupervisionPolicy) -> (Arc<GateKeeper>, Arc<crate::events::EventStore>) {
        let store = crate::events::EventStore::open(None).unwrap();
        let emitter = Emitter::new(store.clone(), "r1");
        (
            Arc::new(GateKeeper::new(emitter, Arc::new(RwLock::new(policy)))),
            store,
        )
    }

    fn checkpoint(stage: Stage, confidence: f64) -> Checkpoint {
        Checkpoint {
            stage,
            task_id: None,
            title: "Validation".into(),
            summary: "résumé".into(),
            proposal: json!({ "kind": "plan" }),
            confidence,
            exceptions: vec![],
            options: GateOption::defaults(),
        }
    }

    #[tokio::test]
    async fn a_gate_parks_the_run_until_a_human_decides() {
        let (gate, store) = keeper(policy(SupervisionMode::Hitl));
        let waiting = tokio::spawn({
            let gate = gate.clone();
            async move { gate.checkpoint(checkpoint(Stage::Plan, 0.9)).await }
        });
        let gate_id = loop {
            if let Some(id) = gate.pending_gate_ids().first() {
                break id.clone();
            }
            tokio::task::yield_now().await;
        };
        assert!(!waiting.is_finished(), "the run must wait for the decision");
        assert!(gate
            .resolve(
                "gate-unknown",
                GateResolution {
                    decision: GateDecision::Approve,
                    feedback: None,
                    resolver: "x".into()
                }
            )
            .is_err());
        gate.resolve(
            &gate_id,
            GateResolution {
                decision: GateDecision::RequestChanges,
                feedback: Some("plus petit".into()),
                resolver: "alice".into(),
            },
        )
        .unwrap();
        assert_eq!(
            waiting.await.unwrap(),
            GateOutcome::ChangesRequested {
                feedback: "plus petit".into()
            }
        );
        // A gate can only be decided once.
        assert!(gate
            .resolve(
                &gate_id,
                GateResolution {
                    decision: GateDecision::Approve,
                    feedback: None,
                    resolver: "bob".into()
                }
            )
            .is_err());

        let kinds: Vec<EventType> = store.events("r1", 0).iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                EventType::GateEvaluated,
                EventType::GateRequested,
                EventType::RunStatusChanged,
                EventType::GateResolved,
                EventType::RunStatusChanged,
            ]
        );
        assert_eq!(store.events("r1", 0)[3].payload["resolver"], "alice");
    }

    #[tokio::test]
    async fn an_unanswered_gate_expires_as_a_rejection_never_as_an_approval() {
        let mut p = policy(SupervisionMode::Hitl);
        p.gate_timeout_s = Some(1);
        let (gate, _) = keeper(p);
        let outcome = gate.checkpoint(checkpoint(Stage::Merge, 0.99)).await;
        // …and it is reported as the factory stopping, not as a human rejection.
        assert!(
            matches!(&outcome, GateOutcome::Rejected { by_human: false, reason } if reason.contains("aucune décision humaine")),
            "{outcome:?}"
        );
        assert!(gate.pending_gate_ids().is_empty());
    }

    #[tokio::test]
    async fn autonomous_checkpoints_never_create_a_gate() {
        let (gate, store) = keeper(policy(SupervisionMode::Autonomous));
        let approved = gate.checkpoint(checkpoint(Stage::Plan, 0.2)).await;
        assert_eq!(
            approved,
            GateOutcome::Approved {
                feedback: None,
                by_human: false
            }
        );
        let mut blocked = checkpoint(Stage::Implementation, 0.9);
        blocked.exceptions.push(ExceptionSignal::new(
            ExceptionKind::TddDivergence,
            "4 itérations",
        ));
        assert!(matches!(
            gate.checkpoint(blocked).await,
            GateOutcome::Rejected {
                by_human: false,
                ..
            }
        ));
        assert!(store
            .events("r1", 0)
            .iter()
            .all(|e| e.kind == EventType::GateEvaluated));
    }

    #[test]
    fn policy_roundtrips_through_json_and_toml() {
        let mut p = policy(SupervisionMode::Hotl);
        p.stage_overrides
            .insert(Stage::TestPlan, SupervisionMode::Hitl);
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<SupervisionPolicy>(&json).unwrap(), p);
        let parsed: SupervisionPolicy =
            toml::from_str("default_mode = \"hotl\"\n[stage_overrides]\ntest_plan = \"hitl\"\n")
                .unwrap();
        assert_eq!(parsed, p);
    }
}
