//! Specialised agents and their runtime.
//!
//! A role is data ([`RoleSpec`]): persona, tools, write scope and the kind of work that
//! drives model routing. One generic runtime executes any role through the same
//! **Observe → Think → Act** loop:
//!
//! 1. *Think* — the model reasons and decides (`agent.thought`);
//! 2. *Act* — it calls tools (`action`);
//! 3. *Observe* — tool results come back (`observation`) and feed the next iteration,
//!
//! until the agent calls `finish` or exhausts its step budget.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

use crate::error::Result;
use crate::events::{Emitter, EventType};
use crate::llm::router::{Budget, ModelRouter};
use crate::llm::{CallMeta, ChatMessage, LlmRequest, TaskKind};
use crate::sandbox::{CommandGuard, Sandbox};
use crate::tools::{self, SearchProvider, ToolContext, WriteScope};

#[derive(Debug, Clone, Serialize)]
pub struct RoleSpec {
    pub id: &'static str,
    pub name: &'static str,
    /// One-line positioning, shown in the dashboard (French UI).
    pub title: &'static str,
    pub mission: &'static str,
    pub inputs: &'static [&'static str],
    pub outputs: &'static [&'static str],
    pub tools: &'static [&'static str],
    pub write_scope: WriteScope,
    /// Default nature of the work, which selects the model tier.
    pub default_kind: TaskKind,
    /// System prompt persona.
    pub persona: &'static str,
}

pub const PRODUCT_OWNER: &str = "product_owner";
pub const ARCHITECT: &str = "architect";
pub const QA_ENGINEER: &str = "qa_engineer";
pub const BACKEND_DEV: &str = "backend_dev";
pub const FRONTEND_DEV: &str = "frontend_dev";
pub const SECURITY_AUDITOR: &str = "security_auditor";
pub const CODE_REVIEWER: &str = "code_reviewer";

pub static ROLES: [RoleSpec; 7] = [
    RoleSpec {
        id: PRODUCT_OWNER,
        name: "Product Owner",
        title: "Porte le « quoi » et le « pourquoi »",
        mission: "Analyse la demande, la décompose en réseau hiérarchique de tâches (HTN), formule des critères d'acceptation testables, cartographie dépendances et risques.",
        inputs: &["Demande brute (user story, epic ou cahier des charges)", "Retours du superviseur sur le plan"],
        outputs: &["Plan HTN : objectif → epics → tâches", "Critères d'acceptation, dépendances, estimations, risques", "Plan de sprints"],
        tools: &["read_file", "list_files", "web_search", "finish"],
        write_scope: WriteScope::None,
        default_kind: TaskKind::Planning,
        persona: "You are the Product Owner of an autonomous software factory. You own the *what* and the *why*: you turn a request into a small set of verifiable outcomes, each with concrete, testable acceptance criteria, real technical dependencies and an honest risk assessment. You never specify the *how*.",
    },
    RoleSpec {
        id: ARCHITECT,
        name: "Architecte logiciel",
        title: "Porte le « comment »",
        mission: "Conçoit l'architecture minimale qui satisfait le plan : découpage en modules, interfaces publiques, modèle de données, stratégie de test, compromis assumés.",
        inputs: &["Plan validé", "Arborescence et code existants du workspace", "Retours du superviseur"],
        outputs: &["docs/ARCHITECTURE.md (composants, interfaces, modèle de données, stratégie de test)", "Niveau de confiance"],
        tools: &["read_file", "list_files", "code_search", "write_file", "web_search", "finish"],
        write_scope: WriteScope::Docs,
        default_kind: TaskKind::Architecture,
        persona: "You are the Software Architect of an autonomous software factory. You own the *how*: given a validated plan you define the smallest architecture that satisfies it — module boundaries, public interfaces with their signatures, data model, technology choices and test strategy. Explore the existing workspace before deciding, prefer boring and proven technology, and record each decision with its rationale and trade-offs so that QA and developers can work independently from your document. Your deliverable is `docs/ARCHITECTURE.md`. You do not write production code.",
    },
    RoleSpec {
        id: QA_ENGINEER,
        name: "Ingénieur QA",
        title: "Écrit les tests avant le code (RED)",
        mission: "Traduit les critères d'acceptation en tests automatisés qui échouent pour la bonne raison tant que la fonctionnalité n'existe pas.",
        inputs: &["Tâche et critères d'acceptation", "Architecture", "Retours du superviseur sur le plan de test"],
        outputs: &["Fichiers de tests (unitaires / intégration)", "Synthèse du plan de test"],
        tools: &["read_file", "list_files", "code_search", "write_file", "str_replace", "run_tests", "finish"],
        write_scope: WriteScope::Tests,
        default_kind: TaskKind::TestGeneration,
        persona: "You are the QA Engineer and you write the tests first (RED phase of test-driven development). From the task's acceptance criteria and the architecture, write automated tests that specify the expected behaviour precisely: one behaviour per test, explicit names, the happy path, edge cases and failure modes. The tests must fail now for the right reason — the feature does not exist yet — and pass once it is correctly implemented, without depending on implementation details. You only write test files. You never write production code and never weaken a test to make it pass.",
    },
    RoleSpec {
        id: BACKEND_DEV,
        name: "Développeur backend",
        title: "Fait passer les tests (GREEN) puis nettoie (REFACTOR)",
        mission: "Implémente la solution minimale et correcte qui fait passer les tests verrouillés, puis corrige lint et findings de sécurité sans casser les tests.",
        inputs: &["Tests en échec", "Architecture", "Diagnostic de la boucle d'auto-correction", "Indications du superviseur"],
        outputs: &["Code de production", "Résumé des changements et niveau de confiance"],
        tools: &["read_file", "list_files", "code_search", "write_file", "str_replace", "bash", "run_tests", "git", "finish"],
        write_scope: WriteScope::Source,
        default_kind: TaskKind::Code,
        persona: "You are the Backend Developer (GREEN phase of test-driven development). Tests already exist and are locked: make them pass by writing the simplest correct production code that follows the architecture. Read the failing tests and any diagnosis first, change production code only, run the tests to check your work, and keep the code clean: clear names, no dead code, explicit error handling, no secrets in code. You never edit, delete or skip tests; if a test looks wrong, say so in your summary and report a low confidence instead.",
    },
    RoleSpec {
        id: FRONTEND_DEV,
        name: "Développeur frontend",
        title: "Fait passer les tests d'interface (GREEN)",
        mission: "Implémente les composants et écrans qui font passer les tests verrouillés, en respectant l'accessibilité et l'architecture.",
        inputs: &["Tests en échec", "Architecture", "Diagnostic de la boucle d'auto-correction", "Indications du superviseur"],
        outputs: &["Composants et écrans", "Résumé des changements et niveau de confiance"],
        tools: &["read_file", "list_files", "code_search", "write_file", "str_replace", "bash", "run_tests", "git", "finish"],
        write_scope: WriteScope::Source,
        default_kind: TaskKind::Code,
        persona: "You are the Frontend Developer (GREEN phase of test-driven development). Tests already exist and are locked: make them pass by writing the simplest correct UI code that follows the architecture — small accessible components, explicit state, no business logic hidden in views. Read the failing tests and any diagnosis first, change production code only, and run the tests to check your work. You never edit, delete or skip tests; if a test looks wrong, say so in your summary and report a low confidence instead.",
    },
    RoleSpec {
        id: SECURITY_AUDITOR,
        name: "Auditeur sécurité",
        title: "Relit le changement comme un attaquant",
        mission: "Audite le diff (OWASP, secrets, injections, cryptographie, authn/authz) en complément du scanner statique déterministe.",
        inputs: &["Diff du run", "Résultats du scanner statique"],
        outputs: &["Findings (sévérité, emplacement, remédiation)", "Avis de risque"],
        tools: &["read_file", "list_files", "code_search", "git", "finish"],
        write_scope: WriteScope::None,
        default_kind: TaskKind::SecurityReview,
        persona: "You are the Security Auditor. You review the change set the way an attacker would: injection, broken authentication and authorisation, secrets in code, cryptographic misuse, unsafe deserialisation, missing input validation, risky dependencies. You have read-only access. Report each issue as a finding with its severity, location, why it is exploitable and how to fix it. Only report issues you can point to in the code — no speculative findings.",
    },
    RoleSpec {
        id: CODE_REVIEWER,
        name: "Relecteur de code",
        title: "Dernier contrôle qualité avant la merge request",
        mission: "Vérifie que le changement répond aux critères d'acceptation, que les tests sont pertinents et que le code est lisible et cohérent avec l'architecture.",
        inputs: &["Diff du run", "Plan et critères d'acceptation", "Architecture"],
        outputs: &["Verdict (approve / request_changes)", "Commentaires actionnables", "Niveau de confiance"],
        tools: &["read_file", "list_files", "code_search", "git", "finish"],
        write_scope: WriteScope::None,
        default_kind: TaskKind::Critique,
        persona: "You are the Code Reviewer, the last quality gate before the merge request. You have read-only access. Check that the change does what the tasks asked (acceptance criteria), that the tests are meaningful, that the code is readable and consistent with the architecture, and that nothing unrelated slipped in. Give a verdict — `approve` or `request_changes` — with precise, actionable comments.",
    },
];

pub fn role(id: &str) -> &'static RoleSpec {
    ROLES.iter().find(|r| r.id == id).unwrap_or(&ROLES[3])
}

const OPERATING_RULES: &str = r#"

## How you work
You operate in an Observe → Think → Act loop: read the latest tool results (observe), reason briefly about the next step (think), then call tools (act). Before each tool call, say in one or two sentences what you are about to do and why.

## Rules
- You act only through the provided tools. The workspace root is the current directory; use relative paths.
- Stay strictly within your assignment and your role. Do not touch anything unrelated.
- Never claim something you have not verified with a tool.
- Write code, comments and identifiers in English unless the existing code does otherwise; write prose deliverables in the language of the request.
- When the assignment is complete — or if you are blocked — call `finish` exactly once, with a summary and an honest confidence between 0 and 1. A low confidence is useful: it routes the work to a human supervisor instead of letting a doubtful result through."#;

/// One unit of work handed to an agent.
#[derive(Clone)]
pub struct Assignment {
    pub role: &'static RoleSpec,
    pub kind: TaskKind,
    /// Workflow step, e.g. `red`, `green`, `refactor`.
    pub phase: String,
    pub instruction: String,
    pub task_id: Option<String>,
    pub attempt: u32,
    pub escalation: u32,
    /// Structured context forwarded in [`CallMeta`].
    pub context: Value,
    pub tests_locked: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AgentResult {
    pub summary: String,
    pub confidence: f64,
    pub verdict: Option<String>,
    pub findings: Vec<Value>,
    pub steps: u32,
    /// `false` when the agent ran out of steps without calling `finish`.
    pub finished: bool,
}

/// Shared execution environment for every agent of a run.
pub struct AgentRuntime {
    pub router: Arc<ModelRouter>,
    pub budget: Arc<Budget>,
    pub emitter: Emitter,
    pub sandbox: Arc<dyn Sandbox>,
    pub max_steps: u32,
    pub exec_timeout: Duration,
    pub test_command: String,
    pub search: Option<Arc<dyn SearchProvider>>,
    pub violations: Arc<Mutex<Vec<String>>>,
}

fn clip(value: &Value, max: usize) -> Value {
    match value {
        Value::String(s) if s.chars().count() > max => Value::String(format!(
            "{}… [{} caractères]",
            s.chars().take(max).collect::<String>(),
            s.chars().count()
        )),
        Value::Object(map) => {
            Value::Object(map.iter().map(|(k, v)| (k.clone(), clip(v, max))).collect())
        }
        Value::Array(items) => Value::Array(items.iter().map(|v| clip(v, max)).collect()),
        other => other.clone(),
    }
}

impl AgentRuntime {
    /// Drains the policy violations recorded since the last call.
    pub fn take_violations(&self) -> Vec<String> {
        std::mem::take(&mut *self.violations.lock().expect("violations poisoned"))
    }

    /// Runs one assignment. The agent holds the "speech token" between the
    /// `agent.turn_started` and `agent.turn_ended` events.
    pub async fn run(&self, assignment: Assignment) -> Result<AgentResult> {
        let source = format!("agent:{}", assignment.role.id);
        let task_id = assignment.task_id.clone();
        self.emitter.emit(
            EventType::AgentTurnStarted,
            &source,
            task_id.as_deref(),
            json!({
                "role": assignment.role.id,
                "name": assignment.role.name,
                "phase": assignment.phase,
                "attempt": assignment.attempt,
                "kind": assignment.kind,
            }),
        );
        let phase = assignment.phase.clone();
        let role_id = assignment.role.id;
        let outcome = self.run_loop(assignment, &source).await;
        let payload = match &outcome {
            Ok(result) => json!({
                "role": role_id,
                "phase": phase,
                "ok": true,
                "finished": result.finished,
                "summary": result.summary,
                "confidence": result.confidence,
                "steps": result.steps,
            }),
            Err(e) => {
                json!({ "role": role_id, "phase": phase, "ok": false, "error": e.to_string() })
            }
        };
        self.emitter.emit(
            EventType::AgentTurnEnded,
            &source,
            task_id.as_deref(),
            payload,
        );
        outcome
    }

    async fn run_loop(&self, a: Assignment, source: &str) -> Result<AgentResult> {
        let allowed: Vec<String> = a.role.tools.iter().map(|t| t.to_string()).collect();
        let ctx = ToolContext {
            role: a.role.id.to_string(),
            task_id: a.task_id.clone(),
            sandbox: self.sandbox.clone(),
            emitter: self.emitter.clone(),
            guard: CommandGuard::shared(),
            write_scope: a.role.write_scope,
            allowed_tools: allowed.clone(),
            tests_locked: a.tests_locked,
            test_command: self.test_command.clone(),
            exec_timeout: self.exec_timeout,
            search: self.search.clone(),
            violations: self.violations.clone(),
        };
        let schemas = tools::schemas_for(&allowed);
        let task_id = a.task_id.as_deref();
        let mut messages = vec![
            ChatMessage::system(format!("{}{}", a.role.persona, OPERATING_RULES)),
            ChatMessage::user(a.instruction.clone()),
        ];
        let mut nudged = false;

        for step in 1..=self.max_steps {
            let meta = CallMeta {
                run_id: self.emitter.run_id().to_string(),
                role: a.role.id.to_string(),
                task_id: a.task_id.clone(),
                phase: a.phase.clone(),
                attempt: a.attempt,
                escalation: a.escalation,
                context: a.context.clone(),
            };
            let request =
                LlmRequest::new(a.kind, messages.clone(), meta).with_tools(schemas.clone());
            let response = self
                .router
                .complete(request, &self.budget, &self.emitter)
                .await?;

            // THINK
            if let Some(thought) = response
                .content
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
            {
                self.emitter.emit(
                    EventType::AgentThought,
                    source,
                    task_id,
                    json!({ "step": step, "text": thought }),
                );
            }

            if response.tool_calls.is_empty() {
                let text = response.content.clone().unwrap_or_default();
                if nudged || text.trim().is_empty() {
                    // The model keeps answering in prose: accept it, with a prudent confidence.
                    return Ok(AgentResult {
                        summary: text,
                        confidence: 0.5,
                        steps: step,
                        finished: !nudged,
                        ..Default::default()
                    });
                }
                nudged = true;
                messages.push(ChatMessage::assistant(Some(text), vec![]));
                messages.push(ChatMessage::user(
                    "Continue with tool calls. If the assignment is complete, call `finish` with your summary and confidence.",
                ));
                continue;
            }

            messages.push(ChatMessage::assistant(
                response.content.clone(),
                response.tool_calls.clone(),
            ));
            let mut finished: Option<AgentResult> = None;
            for call in &response.tool_calls {
                if call.name == "finish" {
                    finished = Some(AgentResult {
                        summary: call.arguments["summary"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        confidence: call.arguments["confidence"]
                            .as_f64()
                            .unwrap_or(0.5)
                            .clamp(0.0, 1.0),
                        verdict: call.arguments["verdict"].as_str().map(str::to_string),
                        findings: call.arguments["findings"]
                            .as_array()
                            .cloned()
                            .unwrap_or_default(),
                        steps: step,
                        finished: true,
                    });
                    messages.push(ChatMessage::tool_result(&call.id, "ok"));
                    continue;
                }
                // ACT
                self.emitter.emit(
                    EventType::Action,
                    source,
                    task_id,
                    json!({ "step": step, "call_id": call.id, "tool": call.name, "args": clip(&call.arguments, 1500) }),
                );
                let output = tools::execute(&call.name, &call.arguments, &ctx).await;
                // OBSERVE
                self.emitter.emit(
                    EventType::Observation,
                    source,
                    task_id,
                    json!({
                        "step": step,
                        "call_id": call.id,
                        "tool": call.name,
                        "ok": output.ok,
                        "output": clip(&Value::String(output.text.clone()), 4000),
                        "data": output.data,
                    }),
                );
                messages.push(ChatMessage::tool_result(&call.id, output.text));
            }
            if let Some(result) = finished {
                return Ok(result);
            }
        }

        Ok(AgentResult {
            summary: format!("budget de {} étapes épuisé sans conclusion", self.max_steps),
            confidence: 0.3,
            steps: self.max_steps,
            finished: false,
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_role_only_references_existing_tools() {
        let known: Vec<String> = tools::catalogue().into_iter().map(|t| t.name).collect();
        for role in ROLES.iter() {
            assert!(role.tools.contains(&"finish"), "{} cannot finish", role.id);
            for tool in role.tools {
                assert!(
                    known.iter().any(|k| k == tool),
                    "{} references unknown tool {tool}",
                    role.id
                );
            }
        }
    }

    #[test]
    fn least_privilege_is_enforced_by_role_definitions() {
        // Read-only roles have no write or shell tool at all.
        for id in [SECURITY_AUDITOR, CODE_REVIEWER, PRODUCT_OWNER] {
            let r = role(id);
            assert_eq!(r.write_scope, WriteScope::None);
            assert!(
                !r.tools.contains(&"bash") && !r.tools.contains(&"write_file"),
                "{id}"
            );
        }
        // QA writes tests but has no shell; only developers get bash.
        assert!(!role(QA_ENGINEER).tools.contains(&"bash"));
        assert_eq!(role(QA_ENGINEER).write_scope, WriteScope::Tests);
        assert!(role(BACKEND_DEV).tools.contains(&"bash"));
    }
}
