//! The mandatory TDD cycle for one task: RED → GREEN → REFACTOR.
//!
//! It is an Evaluator-Optimizer loop where the evaluator is *executable*: the orchestrator
//! runs the tests in the sandbox itself and never takes an agent's word for the verdict.
//!
//! * **RED** — QA writes the tests; they must fail, for the right reason. Vacuous tests
//!   are rejected. Once approved, test files are locked.
//! * **GREEN** — the developer implements; on failure a critic model diagnoses the trace
//!   (self-correction loop) and the developer retries, escalating to stronger models,
//!   until the tests pass or the iteration allowance is spent (→ supervision exception).
//! * **REFACTOR** — lint, fix, and keep the change only if the tests stay green.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::state::TaskStatus;
use super::{rejection, RunCtx};
use crate::agents::{role, Assignment, BACKEND_DEV, FRONTEND_DEV, QA_ENGINEER};
use crate::error::{Error, Result};
use crate::events::EventType;
use crate::governance::{
    Checkpoint, ExceptionKind, ExceptionSignal, GateDecision, GateOption, GateOutcome, Stage,
};
use crate::ingestion::htn::{Component, Plan, TaskNode};
use crate::llm::{extract_json, CallMeta, ChatMessage, LlmRequest, TaskKind};
use crate::sandbox::is_test_path;
use crate::tools::TestReport;

/// What the architecture stage hands to the implementation stage.
#[derive(Debug, Clone, Serialize)]
pub struct Architecture {
    pub summary: String,
    pub document: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TddOutcome {
    pub task_id: String,
    pub title: String,
    /// GREEN attempts needed.
    pub attempts: u32,
    pub confidence: f64,
    pub tests: TestReport,
    /// Files touched by the task (tests and production code).
    pub files: Vec<String>,
    pub refactored: bool,
}

/// Structured diagnosis produced by the self-correction loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reflection {
    pub root_cause: String,
    pub fix_plan: String,
    #[serde(default)]
    pub confidence: f64,
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        format!(
            "{}\n… [truncated]",
            text.chars().take(max).collect::<String>()
        )
    }
}

fn task_brief(task: &TaskNode, architecture: &Architecture) -> String {
    let criteria = task
        .acceptance_criteria
        .iter()
        .map(|c| format!("- {c}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "## Task {} — {}\n{}\n\n### Acceptance criteria\n{}\n\n### Architecture (excerpt)\n{}\n",
        task.id,
        task.title,
        task.description,
        criteria,
        clip(&architecture.document, 6000),
    )
}

fn file_listing(ctx: &RunCtx) -> String {
    let files = ctx.ws().list();
    if files.is_empty() {
        return "(empty workspace)".into();
    }
    files
        .iter()
        .take(200)
        .map(|f| f.path.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Why a RED run is not acceptable, if it is not.
fn validate_red(report: &TestReport, new_tests: &[String]) -> std::result::Result<(), String> {
    if new_tests.is_empty() {
        return Err("aucun fichier de test n'a été écrit ou modifié".into());
    }
    if report.timed_out {
        return Err("les tests ne se terminent pas (délai dépassé)".into());
    }
    match report.exit_code {
        0 => Err("les nouveaux tests passent avant toute implémentation : ils ne vérifient rien, ou la fonctionnalité existe déjà".into()),
        5 => Err("aucun test n'a été collecté par le lanceur de tests".into()),
        _ if report.output.contains("SyntaxError") || report.output.contains("IndentationError") => {
            Err("les tests contiennent une erreur de syntaxe".into())
        }
        _ => Ok(()),
    }
}

/// Production files a critic needs to see to diagnose a failure.
fn source_excerpt(ctx: &RunCtx) -> String {
    let mut out = String::new();
    for file in ctx
        .ws()
        .list()
        .into_iter()
        .filter(|f| !is_test_path(&f.path) && !f.path.starts_with("docs/"))
        .take(8)
    {
        if file.size > 40_000 || file.path.starts_with('.') {
            continue;
        }
        if let Ok(content) = ctx.ws().read(&file.path) {
            if !content.trim().is_empty() {
                out.push_str(&format!(
                    "### {}\n```\n{}\n```\n",
                    file.path,
                    clip(&content, 6000)
                ));
            }
        }
    }
    out
}

const REFLECT_SYSTEM: &str = r#"You are the evaluator of a test-driven self-correction loop.
A developer agent tried to make a locked test-suite pass and failed. Analyse the failing
output and the production code, and identify the root cause in the PRODUCTION code — the
tests are the specification and must not change.

Answer with a single JSON object and nothing else:
{"root_cause": "what is wrong and where (file, function)", "fix_plan": "the precise change to make", "confidence": 0.0-1.0}"#;

/// Self-correction: a critic model (different tier — and ideally vendor — than the
/// developer) turns a failing trace into a diagnosis.
async fn reflect(
    ctx: &RunCtx,
    task: &TaskNode,
    report: &TestReport,
    attempt: u32,
    context: &Value,
) -> Result<Reflection> {
    let source = format!("agent:{QA_ENGINEER}");
    ctx.emitter.emit(
        EventType::AgentTurnStarted,
        &source,
        Some(&task.id),
        json!({ "role": QA_ENGINEER, "name": role(QA_ENGINEER).name, "phase": "reflect", "attempt": attempt, "kind": TaskKind::Critique }),
    );
    let user = format!(
        "## Task {} — {}\n\n## Failing test run (attempt {attempt})\n```\n{}\n```\n\n## Production code\n{}",
        task.id,
        task.title,
        clip(&report.output, 8000),
        source_excerpt(ctx),
    );
    let meta = CallMeta {
        run_id: ctx.run_id.clone(),
        role: QA_ENGINEER.into(),
        task_id: Some(task.id.clone()),
        phase: "reflect".into(),
        attempt,
        escalation: 0,
        context: context.clone(),
    };
    let request = LlmRequest::new(
        TaskKind::Critique,
        vec![ChatMessage::system(REFLECT_SYSTEM), ChatMessage::user(user)],
        meta,
    )
    .json();
    let outcome = ctx.complete(request).await.map(|response| {
        let text = response.content.unwrap_or_default();
        extract_json(&text)
            .and_then(|v| serde_json::from_value::<Reflection>(v).ok())
            .unwrap_or(Reflection {
                root_cause: clip(&text, 1500),
                fix_plan: String::new(),
                confidence: 0.4,
            })
    });
    let payload = match &outcome {
        Ok(r) => {
            json!({ "role": QA_ENGINEER, "phase": "reflect", "ok": true, "finished": true, "summary": r.root_cause, "confidence": r.confidence, "steps": 1 })
        }
        Err(e) => {
            json!({ "role": QA_ENGINEER, "phase": "reflect", "ok": false, "error": e.to_string() })
        }
    };
    ctx.emitter
        .emit(EventType::AgentTurnEnded, &source, Some(&task.id), payload);
    let reflection = outcome?;
    ctx.emitter.emit(
        EventType::TddReflection,
        &source,
        Some(&task.id),
        json!({ "attempt": attempt, "root_cause": reflection.root_cause, "fix_plan": reflection.fix_plan, "confidence": reflection.confidence }),
    );
    Ok(reflection)
}

/// Runs the full TDD cycle for one task.
pub(crate) async fn run_task(
    ctx: &RunCtx,
    plan: &Plan,
    task: &TaskNode,
    architecture: &Architecture,
) -> Result<TddOutcome> {
    let tid = task.id.as_str();
    let policy = ctx.policy();
    let dev_role = role(if task.component == Component::Frontend {
        FRONTEND_DEV
    } else {
        BACKEND_DEV
    });
    let brief = task_brief(task, architecture);
    let mut context = ctx.base_context();
    context["task"] = serde_json::to_value(task)?;
    context["task_count"] = json!(plan.execution_order.len());
    context["goal"] = json!(plan.goal);
    let files_before: BTreeMap<String, String> = ctx
        .ws()
        .list()
        .into_iter()
        .filter_map(|f| ctx.ws().read(&f.path).ok().map(|c| (f.path, c)))
        .collect();

    // ------------------------------------------------------------------ RED
    ctx.task_status(tid, TaskStatus::Red, 0, None);
    let mut supervisor_feedback: Option<String> = None;
    let mut revision = 0;
    let qa_confidence = loop {
        revision += 1;
        ctx.tdd_phase(tid, "red", revision, policy.max_revisions + 1);
        let tests_before = ctx.snapshot_tests();
        let mut note = supervisor_feedback.take().map(|f| {
            format!("A human supervisor reviewed your test plan and requested changes:\n{f}")
        });
        let mut exceptions = Vec::new();
        let mut tries = 0;
        let (result, report, new_tests) = loop {
            tries += 1;
            let instruction = format!(
                "{brief}\n### Workspace files\n{}\n\n### Your assignment (RED phase)\nWrite the automated tests for this task, in test files only. They must fail now — the feature is not implemented yet — and pass once it is. Do not write production code.\n{}",
                file_listing(ctx),
                note.as_deref().unwrap_or(""),
            );
            let result = ctx
                .run_agent(Assignment {
                    role: role(QA_ENGINEER),
                    kind: TaskKind::TestGeneration,
                    phase: "red".into(),
                    instruction,
                    task_id: Some(task.id.clone()),
                    attempt: revision,
                    escalation: 0,
                    context: context.clone(),
                    tests_locked: false,
                })
                .await?;
            let tests_after = ctx.snapshot_tests();
            let new_tests: Vec<String> = tests_after
                .iter()
                .filter(|(p, c)| tests_before.get(*p) != Some(*c))
                .map(|(p, _)| p.clone())
                .collect();
            // Only the new tests are run: they are the ones that must fail.
            let report = ctx
                .verify_tests(Some(tid), "red", tries, "fail", &new_tests)
                .await?;
            match validate_red(&report, &new_tests) {
                Ok(()) => break (result, report, new_tests),
                Err(reason) if tries < 2 => {
                    ctx.emitter.log(
                        "orchestrator",
                        "warn",
                        format!("RED invalide ({reason}) : nouvelle tentative de l'agent QA"),
                    );
                    note = Some(format!("Your previous tests were rejected by the orchestrator: {reason}. Fix them."));
                }
                Err(reason) => {
                    exceptions.push(ExceptionSignal::new(
                        ExceptionKind::VacuousTests,
                        format!("phase RED invalide : {reason}"),
                    ));
                    break (result, report, new_tests);
                }
            }
        };
        ctx.commit(&format!("test(red): {} — {}", task.id, task.title))
            .await?;

        let test_files: Vec<Value> = new_tests
            .iter()
            .map(|path| json!({ "path": path, "content": ctx.ws().read(path).unwrap_or_default() }))
            .collect();
        let outcome = ctx
            .checkpoint(Checkpoint {
                stage: Stage::TestPlan,
                task_id: Some(task.id.clone()),
                title: format!("Plan de test — {}", task.title),
                summary: result.summary.clone(),
                proposal: json!({
                    "kind": "test_plan",
                    "task": task,
                    "files": test_files,
                    "red_run": { "summary": report.summary(), "output": clip(&report.output, 4000) },
                }),
                confidence: result.confidence,
                exceptions,
                options: GateOption::defaults(),
            })
            .await;
        match outcome {
            GateOutcome::Approved { .. } => break result.confidence,
            GateOutcome::ChangesRequested { feedback } if revision <= policy.max_revisions => {
                supervisor_feedback = Some(feedback)
            }
            GateOutcome::ChangesRequested { .. } => {
                ctx.task_status(tid, TaskStatus::Failed, 0, None);
                return Err(Error::Aborted(format!(
                    "plan de test de {} : nombre maximal de révisions atteint",
                    task.id
                )));
            }
            GateOutcome::Rejected { reason, by_human } => {
                ctx.task_status(tid, TaskStatus::Failed, 0, None);
                return Err(rejection(
                    &format!("plan de test de {}", task.id),
                    reason,
                    by_human,
                ));
            }
        }
    };
    // From here on, test files are frozen for developers.
    let locked_tests = ctx.snapshot_tests();

    // ---------------------------------------------------------------- GREEN
    ctx.task_status(tid, TaskStatus::Green, 0, None);
    let mut attempt = 0;
    let mut allowance = policy.max_tdd_iterations;
    let mut reflection: Option<Reflection> = None;
    let mut supervisor_hint: Option<String> = None;
    let mut dev_confidence;
    let green_report = loop {
        attempt += 1;
        ctx.tdd_phase(tid, "green", attempt, allowance);
        // Repeated failures climb the model ladder (e.g. coder → reasoning).
        let escalation = (attempt - 1) / ctx.cfg.agents.escalate_after_failures.max(1);
        let kind = if task.risk.score >= 12 {
            TaskKind::ComplexCode
        } else {
            TaskKind::Code
        };
        let mut instruction = format!(
            "{brief}\n### Workspace files\n{}\n\n### Your assignment (GREEN phase, attempt {attempt})\nThe tests for this task exist and are locked. Write the simplest correct production code that makes the whole test-suite pass. Run the tests before you finish.\n",
            file_listing(ctx),
        );
        if let Some(r) = &reflection {
            instruction.push_str(&format!(
                "\n### Diagnosis of your previous attempt (self-correction loop)\nRoot cause: {}\nFix plan: {}\n",
                r.root_cause, r.fix_plan
            ));
        }
        let lessons = ctx.memory.search(
            &ctx.run_id,
            Some("lesson"),
            &format!("{} {}", task.title, task.description),
            2,
        );
        if !lessons.is_empty() {
            instruction.push_str("\n### Lessons learned earlier in this run\n");
            for (_, lesson) in &lessons {
                instruction.push_str(&format!("- {}\n", lesson.text));
            }
        }
        if let Some(hint) = &supervisor_hint {
            instruction.push_str(&format!(
                "\n### Guidance from the human supervisor\n{hint}\n"
            ));
        }
        let result = ctx
            .run_agent(Assignment {
                role: dev_role,
                kind,
                phase: "green".into(),
                instruction,
                task_id: Some(task.id.clone()),
                attempt,
                escalation,
                context: context.clone(),
                tests_locked: true,
            })
            .await?;
        dev_confidence = result.confidence;
        ctx.restore_locked_tests(&locked_tests, tid);

        // The verdict belongs to the orchestrator: full suite, in the sandbox.
        let report = ctx
            .verify_tests(Some(tid), "green", attempt, "pass", &[])
            .await?;
        if report.ok {
            break report;
        }
        ctx.task_status(tid, TaskStatus::Green, attempt, None);

        if attempt >= allowance {
            let outcome = ctx
                .checkpoint(Checkpoint {
                    stage: Stage::Implementation,
                    task_id: Some(task.id.clone()),
                    title: format!("Divergence TDD — {}", task.title),
                    summary: format!("Les tests échouent encore après {attempt} tentative(s) : {}", report.summary()),
                    proposal: json!({
                        "kind": "tdd_divergence",
                        "task": task,
                        "attempts": attempt,
                        "last_run": { "summary": report.summary(), "output": clip(&report.output, 6000) },
                        "last_reflection": reflection,
                    }),
                    confidence: dev_confidence,
                    exceptions: vec![ExceptionSignal::new(
                        ExceptionKind::TddDivergence,
                        format!("tests toujours rouges après {attempt} itération(s) sur {}", task.id),
                    )],
                    options: vec![
                        GateOption::new(GateDecision::Approve, &format!("Accorder {} itérations supplémentaires", policy.max_tdd_iterations)),
                        GateOption::new(GateDecision::RequestChanges, "Réessayer avec mes indications"),
                        GateOption::new(GateDecision::Reject, "Abandonner et arrêter le run"),
                    ],
                })
                .await;
            match outcome {
                GateOutcome::Approved { feedback, .. } => {
                    allowance += policy.max_tdd_iterations;
                    supervisor_hint = feedback.or(supervisor_hint);
                }
                GateOutcome::ChangesRequested { feedback } => {
                    allowance += policy.max_tdd_iterations;
                    supervisor_hint = Some(feedback);
                }
                GateOutcome::Rejected { reason, by_human } => {
                    ctx.task_status(tid, TaskStatus::Failed, attempt, None);
                    return Err(rejection(&format!("tâche {}", task.id), reason, by_human));
                }
            }
        }

        let diagnosis = reflect(ctx, task, &report, attempt, &context).await?;
        ctx.memory.add(
            &ctx.run_id,
            "lesson",
            &format!(
                "{} — {} Correctif : {}",
                task.title, diagnosis.root_cause, diagnosis.fix_plan
            ),
        );
        reflection = Some(diagnosis);
    };
    ctx.commit(&format!("feat(green): {} — {}", task.id, task.title))
        .await?;

    // ------------------------------------------------------------- REFACTOR
    ctx.task_status(tid, TaskStatus::Refactor, attempt, None);
    ctx.tdd_phase(tid, "refactor", 1, 1);
    let mut final_report = green_report;
    let mut refactored = false;
    let lint = ctx.sh(&ctx.cfg.sandbox.lint_command).await?;
    if !lint.ok() {
        let instruction = format!(
            "{brief}\n### Your assignment (REFACTOR phase)\nThe tests pass. The linter reports the issues below: fix them without changing behaviour. Tests are locked and must stay green.\n```\n{}\n```\n",
            clip(&lint.combined(), 6000),
        );
        // Routine work: routed to the cheapest tier (local open-source model).
        ctx.run_agent(Assignment {
            role: dev_role,
            kind: TaskKind::LintFix,
            phase: "refactor".into(),
            instruction,
            task_id: Some(task.id.clone()),
            attempt: 1,
            escalation: 0,
            context: context.clone(),
            tests_locked: true,
        })
        .await?;
        ctx.restore_locked_tests(&locked_tests, tid);
        let report = ctx
            .verify_tests(Some(tid), "refactor", 1, "pass", &[])
            .await?;
        if report.ok {
            ctx.commit(&format!("refactor: {} — lint", task.id)).await?;
            final_report = report;
            refactored = true;
        } else {
            // A refactor that breaks the tests is thrown away: back to the green commit.
            ctx.sh("git checkout -q -- . && git clean -fdq").await?;
            ctx.emitter.log(
                "orchestrator",
                "warn",
                format!("{} : refactor annulé, il cassait les tests", task.id),
            );
        }
    }

    let files: Vec<String> = ctx
        .ws()
        .list()
        .into_iter()
        .filter(|f| ctx.ws().read(&f.path).ok().as_ref() != files_before.get(&f.path))
        .map(|f| f.path)
        .collect();
    // Objective signal (iterations needed) blended with the agents' self-assessment.
    let objective = (0.95 - 0.12 * f64::from(attempt - 1)).max(0.5);
    let confidence = 0.6 * objective + 0.25 * dev_confidence + 0.15 * qa_confidence;
    ctx.task_status(tid, TaskStatus::Done, attempt, Some(confidence));
    ctx.tdd_phase(tid, "done", attempt, allowance);
    Ok(TddOutcome {
        task_id: task.id.clone(),
        title: task.title.clone(),
        attempts: attempt,
        confidence,
        tests: final_report,
        files,
        refactored,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(exit_code: i32, output: &str) -> TestReport {
        TestReport {
            ok: exit_code == 0,
            exit_code,
            passed: 0,
            failed: 0,
            errors: 0,
            timed_out: false,
            duration_ms: 1,
            command: "pytest".into(),
            output: output.into(),
        }
    }

    #[test]
    fn red_must_fail_for_the_right_reason() {
        let tests = vec!["tests/test_a.py".to_string()];
        assert!(validate_red(&report(1, "1 failed"), &tests).is_ok());
        // Collection error because the module does not exist yet: a legitimate RED.
        assert!(validate_red(
            &report(2, "ModuleNotFoundError: No module named 'app'"),
            &tests
        )
        .is_ok());
        assert!(validate_red(&report(0, "3 passed"), &tests).is_err());
        assert!(validate_red(&report(5, "no tests ran"), &tests).is_err());
        assert!(validate_red(&report(2, "SyntaxError: invalid syntax"), &tests).is_err());
        assert!(validate_red(&report(1, "1 failed"), &[]).is_err());
    }
}
