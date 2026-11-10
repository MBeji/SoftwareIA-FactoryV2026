//! The agentic SDLC workflow, as an explicit state machine:
//!
//! ```text
//! INGESTION → PLANNING → ARCHITECTURE → IMPLEMENTATION (TDD per task) → AUDIT → DELIVERY
//!     │           │            │                 │                        │         │
//!  classify    gate:plan  gate:architecture  gate:test_plan          gate:security gate:merge
//!                                            gate:implementation (exception only)
//! ```
//!
//! Each arrow is *prompt chaining*; the input level *routes* the planning workflow; the
//! audit stage *parallelises* independent reviewers; every gate goes through the
//! supervision matrix.

use serde_json::{json, Value};

use super::state::Phase;
use super::tdd::{self, Architecture, TddOutcome};
use super::{rejection, RunCtx};
use crate::agents::{
    role, AgentResult, Assignment, ARCHITECT, BACKEND_DEV, CODE_REVIEWER, PRODUCT_OWNER,
    SECURITY_AUDITOR,
};
use crate::error::{Error, Result};
use crate::events::EventType;
use crate::governance::security::{blocking, scan_workspace, Finding};
use crate::governance::{
    Checkpoint, ExceptionKind, ExceptionSignal, GateDecision, GateOption, GateOutcome, Severity,
    Stage,
};
use crate::ingestion::htn::{Plan, TaskNode};
use crate::ingestion::planner::{heuristic_plan, parse_llm_plan, planning_messages};
use crate::ingestion::{classify, Classification};
use crate::llm::{CallMeta, LlmRequest, TaskKind};
use crate::orchestrator::state::RunStatus;

const BASE_BRANCH: &str = "main";
const GITIGNORE: &str =
    "__pycache__/\n*.pyc\n.pytest_cache/\n.ruff_cache/\n.venv/\nnode_modules/\ndist/\n.env\n";

pub(crate) async fn execute(ctx: &RunCtx) -> Result<()> {
    ctx.emitter.emit(
        EventType::RunStatusChanged,
        "orchestrator",
        None,
        json!({ "status": RunStatus::Running }),
    );

    // 1. INGESTION -------------------------------------------------------------------
    ctx.set_phase(Phase::Ingestion);
    let branch = prepare_workspace(ctx).await?;
    let mut classification = classify(&ctx.input, ctx.filename.as_deref());
    if let Some(level) = ctx.level_override {
        classification.level = level;
        classification.rationale = format!("niveau {level} imposé par l'utilisateur");
    }
    ctx.emitter.emit(
        EventType::InputClassified,
        "orchestrator",
        None,
        serde_json::to_value(&classification)?,
    );

    // 2. PLANNING --------------------------------------------------------------------
    ctx.set_phase(Phase::Planning);
    let plan = plan_stage(ctx, &classification).await?;

    // 3. ARCHITECTURE ----------------------------------------------------------------
    ctx.set_phase(Phase::Architecture);
    let architecture = architecture_stage(ctx, &plan).await?;

    // 4. IMPLEMENTATION: one TDD cycle per task, in dependency order -----------------
    ctx.set_phase(Phase::Implementation);
    let mut outcomes = Vec::new();
    for task_id in &plan.execution_order {
        let task = plan
            .node(task_id)
            .ok_or_else(|| Error::Other(format!("plan has no task '{task_id}'")))?;
        outcomes.push(tdd::run_task(ctx, &plan, task, &architecture).await?);
    }

    // 5. AUDIT -----------------------------------------------------------------------
    ctx.set_phase(Phase::Audit);
    let audit = audit_stage(ctx, &plan, &outcomes).await?;

    // 6. DELIVERY --------------------------------------------------------------------
    ctx.set_phase(Phase::Delivery);
    delivery_stage(ctx, &plan, &outcomes, &audit, &branch).await?;
    ctx.set_phase(Phase::Done);
    Ok(())
}

/// Starts the sandbox and initialises the git repository the run works in.
async fn prepare_workspace(ctx: &RunCtx) -> Result<String> {
    ctx.sandbox.start().await?;
    let branch = format!("omniforge/{}", ctx.run_id);
    ctx.ws().write(".gitignore", GITIGNORE)?;
    let init = ctx
        .sh(&format!(
            "git init -q -b {BASE_BRANCH} && git config user.name 'OmniForge AI' && git config user.email 'omniforge@localhost' \
             && git config commit.gpgsign false && git add -A && git commit -q -m 'chore: initialise workspace' \
             && git checkout -q -b {branch}"
        ))
        .await?;
    if !init.ok() {
        return Err(Error::Sandbox(format!(
            "cannot initialise the workspace repository: {}",
            init.combined()
        )));
    }
    Ok(branch)
}

fn leaves_json(plan: &Plan) -> Vec<Value> {
    plan.execution_order
        .iter()
        .filter_map(|id| plan.node(id))
        .filter_map(|n| serde_json::to_value(n).ok())
        .collect()
}

/// Product Owner: structured planning call, validated and scheduled deterministically.
async fn plan_stage(ctx: &RunCtx, classification: &Classification) -> Result<Plan> {
    let policy = ctx.policy();
    let capacity = ctx.cfg.planning.sprint_capacity_points;
    let source = format!("agent:{PRODUCT_OWNER}");
    let mut feedback: Option<String> = None;
    let mut revision = 0;
    loop {
        revision += 1;
        ctx.emitter.emit(
            EventType::AgentTurnStarted,
            &source,
            None,
            json!({ "role": PRODUCT_OWNER, "name": role(PRODUCT_OWNER).name, "phase": "plan", "attempt": revision, "kind": TaskKind::Planning }),
        );
        let mut context = ctx.base_context();
        context["input"] = json!(ctx.input);
        context["level"] = json!(classification.level);
        context["sprint_capacity"] = json!(capacity);
        let meta = CallMeta {
            run_id: ctx.run_id.clone(),
            role: PRODUCT_OWNER.into(),
            task_id: None,
            phase: "plan".into(),
            attempt: revision,
            escalation: 0,
            context,
        };
        let messages = planning_messages(&ctx.input, classification.level, feedback.as_deref());
        let planned = match ctx
            .complete(LlmRequest::new(TaskKind::Planning, messages, meta).json())
            .await
        {
            Ok(response) => Ok(
                match parse_llm_plan(
                    &response.content.unwrap_or_default(),
                    classification.level,
                    capacity,
                ) {
                    Ok(mut plan) => {
                        if ctx.router.backend_name() == "simulated" {
                            plan.planner = "heuristic".into();
                        }
                        plan
                    }
                    Err(e) => {
                        // An unusable model plan never blocks the run: fall back to the
                        // deterministic planner, with a confidence that reflects it.
                        ctx.emitter.log("orchestrator", "warn", format!("plan du modèle inutilisable ({e}) : repli sur le planificateur heuristique"));
                        let mut plan = heuristic_plan(&ctx.input, classification.level, capacity);
                        plan.confidence = plan.confidence.min(0.6);
                        plan
                    }
                },
            ),
            Err(e) => Err(e),
        };
        let payload = match &planned {
            Ok(plan) => json!({
                "role": PRODUCT_OWNER, "phase": "plan", "ok": true, "finished": true, "steps": 1,
                "summary": format!("{} tâche(s), {} point(s), {} sprint(s)", plan.execution_order.len(), plan.total_points, plan.sprints.len()),
                "confidence": plan.confidence,
            }),
            Err(e) => {
                json!({ "role": PRODUCT_OWNER, "phase": "plan", "ok": false, "error": e.to_string() })
            }
        };
        ctx.emitter
            .emit(EventType::AgentTurnEnded, &source, None, payload);
        let plan = planned?;
        ctx.emitter.emit(
            EventType::PlanCreated,
            &source,
            None,
            json!({ "plan": plan, "revision": revision }),
        );

        let high_risk = plan
            .leaves()
            .iter()
            .filter(|n| n.risk.level() == "high")
            .count();
        let outcome = ctx
            .checkpoint(Checkpoint {
                stage: Stage::Plan,
                task_id: None,
                title: "Validation du plan".into(),
                summary: format!(
                    "{} tâche(s) en {} vague(s), {} point(s), {} sprint(s), {} tâche(s) à risque élevé.",
                    plan.execution_order.len(),
                    plan.waves.len(),
                    plan.total_points,
                    plan.sprints.len(),
                    high_risk
                ),
                proposal: json!({ "kind": "plan", "plan": plan }),
                confidence: plan.confidence,
                exceptions: vec![],
                options: GateOption::defaults(),
            })
            .await;
        match outcome {
            GateOutcome::Approved { .. } => return Ok(plan),
            GateOutcome::ChangesRequested { feedback: f } if revision <= policy.max_revisions => {
                feedback = Some(f)
            }
            GateOutcome::ChangesRequested { .. } => {
                return Err(Error::Aborted(
                    "plan : nombre maximal de révisions atteint".into(),
                ))
            }
            GateOutcome::Rejected { reason, by_human } => {
                return Err(rejection("plan", reason, by_human))
            }
        }
    }
}

/// Architect: explores the workspace and writes `docs/ARCHITECTURE.md`.
async fn architecture_stage(ctx: &RunCtx, plan: &Plan) -> Result<Architecture> {
    let policy = ctx.policy();
    let mut feedback: Option<String> = None;
    let mut revision = 0;
    loop {
        revision += 1;
        let mut context = ctx.base_context();
        context["goal"] = json!(plan.goal);
        context["tasks"] = json!(leaves_json(plan));
        context["task_count"] = json!(plan.execution_order.len());
        let mut instruction = format!(
            "## Goal\n{}\n\n## Validated plan\n{}\n## Your assignment\nDesign the architecture for this plan and write it to `docs/ARCHITECTURE.md`: context, decisions with rationale and trade-offs, components and their responsibilities, file layout, public interfaces (signatures), data model, test strategy, risks and open questions. Explore the workspace first. Be concrete enough for QA to write tests and developers to implement independently.\n",
            plan.goal,
            plan.outline(),
        );
        if let Some(f) = &feedback {
            instruction.push_str(&format!(
                "\n## Changes requested by the human supervisor\n{f}\n"
            ));
        }
        let result = ctx
            .run_agent(Assignment {
                role: role(ARCHITECT),
                kind: TaskKind::Architecture,
                phase: "architecture".into(),
                instruction,
                task_id: None,
                attempt: revision,
                escalation: 0,
                context,
                tests_locked: false,
            })
            .await?;
        let written = ctx
            .ws()
            .read("docs/ARCHITECTURE.md")
            .ok()
            .filter(|d| !d.trim().is_empty());
        // No document, or no explicit conclusion: the proposal is not trustworthy as is.
        let confidence = if written.is_some() && result.finished {
            result.confidence
        } else {
            result.confidence.min(0.4)
        };
        let document = written.unwrap_or_else(|| result.summary.clone());
        ctx.emitter.emit(
            EventType::ArtifactCreated,
            &format!("agent:{ARCHITECT}"),
            None,
            json!({ "kind": "architecture", "title": "Architecture", "path": "docs/ARCHITECTURE.md", "content": document, "revision": revision }),
        );
        ctx.commit("docs: architecture").await?;

        let outcome = ctx
            .checkpoint(Checkpoint {
                stage: Stage::Architecture,
                task_id: None,
                title: "Validation de l'architecture".into(),
                summary: result.summary.clone(),
                proposal: json!({ "kind": "architecture", "document": document }),
                confidence,
                exceptions: vec![],
                options: GateOption::defaults(),
            })
            .await;
        match outcome {
            GateOutcome::Approved { .. } => {
                return Ok(Architecture {
                    summary: result.summary,
                    document,
                })
            }
            GateOutcome::ChangesRequested { feedback: f } if revision <= policy.max_revisions => {
                feedback = Some(f)
            }
            GateOutcome::ChangesRequested { .. } => {
                return Err(Error::Aborted(
                    "architecture : nombre maximal de révisions atteint".into(),
                ))
            }
            GateOutcome::Rejected { reason, by_human } => {
                return Err(rejection("architecture", reason, by_human))
            }
        }
    }
}

pub(crate) struct AuditOutcome {
    /// Findings still open at the end of the stage.
    pub findings: Vec<Finding>,
    /// A human accepted the remaining blocking findings.
    pub waived: bool,
    pub review: AgentResult,
}

fn severity_from(text: &str) -> Severity {
    match text.to_ascii_lowercase().as_str() {
        "critical" => Severity::Critical,
        "high" => Severity::High,
        "medium" => Severity::Medium,
        "low" => Severity::Low,
        _ => Severity::Info,
    }
}

/// Findings reported by the LLM auditor, normalised.
fn auditor_findings(result: &AgentResult) -> Vec<Finding> {
    result
        .findings
        .iter()
        .filter_map(|f| {
            let message = f["message"].as_str()?.to_string();
            Some(Finding {
                rule_id: "AUDIT".into(),
                severity: severity_from(f["severity"].as_str().unwrap_or("info")),
                path: f["path"].as_str().unwrap_or_default().to_string(),
                line: f["line"].as_u64().unwrap_or(0) as usize,
                message,
                remediation: f["remediation"].as_str().unwrap_or_default().to_string(),
                snippet: String::new(),
            })
        })
        .collect()
}

fn audit_assignment(
    ctx: &RunCtx,
    plan: &Plan,
    role_id: &'static str,
    phase: &str,
    kind: TaskKind,
    instruction: String,
) -> Assignment {
    let mut context = ctx.base_context();
    context["goal"] = json!(plan.goal);
    Assignment {
        role: role(role_id),
        kind,
        phase: phase.into(),
        instruction,
        task_id: None,
        attempt: 1,
        escalation: 0,
        context,
        tests_locked: true,
    }
}

async fn run_auditor(ctx: &RunCtx, plan: &Plan, scanner: &[Finding]) -> Result<AgentResult> {
    let known = if scanner.is_empty() {
        "The static scanner reported nothing.".to_string()
    } else {
        scanner
            .iter()
            .map(|f| {
                format!(
                    "- [{:?}] {}:{} {} ({})",
                    f.severity, f.path, f.line, f.message, f.rule_id
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let instruction = format!(
        "## Goal of the change\n{}\n\n## Static scanner results (already known — do not repeat them)\n{known}\n\n## Your assignment\nAudit the change set on branch HEAD against `{BASE_BRANCH}` (`git diff {BASE_BRANCH}`). Report additional security issues as findings in `finish`, each with severity, path, line, message and remediation. If the change is clean, say so.\n",
        plan.goal
    );
    ctx.run_agent(audit_assignment(
        ctx,
        plan,
        SECURITY_AUDITOR,
        "security_review",
        TaskKind::SecurityReview,
        instruction,
    ))
    .await
}

/// Audit stage. Parallelisation pattern: the security auditor and the code reviewer are
/// read-only and independent, so they review the same change set concurrently.
async fn audit_stage(ctx: &RunCtx, plan: &Plan, outcomes: &[TddOutcome]) -> Result<AuditOutcome> {
    let policy = ctx.policy();
    let scanner = scan_workspace(ctx.ws());
    let review_instruction = format!(
        "## Goal\n{}\n\n## Plan\n{}\n## Your assignment\nReview the change set on branch HEAD against `{BASE_BRANCH}` (`git log --oneline {BASE_BRANCH}..HEAD`, `git diff {BASE_BRANCH}`). Check the acceptance criteria, the relevance of the tests, readability and consistency with `docs/ARCHITECTURE.md`. Conclude with `finish` and a verdict: `approve` or `request_changes`.\n",
        plan.goal,
        plan.outline()
    );
    let (audit, review) = tokio::join!(
        run_auditor(ctx, plan, &scanner),
        ctx.run_agent(audit_assignment(
            ctx,
            plan,
            CODE_REVIEWER,
            "review",
            TaskKind::Critique,
            review_instruction
        )),
    );
    let (audit, mut review) = (audit?, review?);

    let mut llm_findings = auditor_findings(&audit);
    let mut findings: Vec<Finding> = scanner.into_iter().chain(llm_findings.clone()).collect();
    let mut round = 0;
    let mut waived = false;
    let mut supervisor_hint: Option<String> = None;
    loop {
        let open = blocking(&findings, policy.block_on_severity);
        ctx.emitter.emit(
            EventType::SecurityFindings,
            "orchestrator",
            None,
            json!({ "round": round, "findings": findings, "blocking": open.len(), "threshold": policy.block_on_severity }),
        );
        if open.is_empty() {
            break;
        }
        if round >= ctx.cfg.agents.max_security_fix_rounds {
            let outcome = ctx
                .checkpoint(Checkpoint {
                    stage: Stage::Security,
                    task_id: None,
                    title: "Findings de sécurité non résolus".into(),
                    summary: format!("{} finding(s) bloquant(s) subsistent après {round} tour(s) de correction automatique.", open.len()),
                    proposal: json!({ "kind": "security", "findings": open }),
                    confidence: 0.3,
                    exceptions: vec![ExceptionSignal::new(
                        ExceptionKind::SecurityViolation,
                        format!("{} finding(s) de sévérité ≥ {:?}", open.len(), policy.block_on_severity),
                    )],
                    options: vec![
                        GateOption::new(GateDecision::Approve, "Accepter le risque (dérogation tracée dans la PR)"),
                        GateOption::new(GateDecision::RequestChanges, "Relancer une correction avec mes indications"),
                        GateOption::new(GateDecision::Reject, "Arrêter le run"),
                    ],
                })
                .await;
            match outcome {
                GateOutcome::Approved { .. } => {
                    waived = true;
                    break;
                }
                GateOutcome::ChangesRequested { feedback } => supervisor_hint = Some(feedback),
                GateOutcome::Rejected { reason, by_human } => {
                    return Err(rejection("audit de sécurité", reason, by_human))
                }
            }
        }
        round += 1;

        // One fix assignment per task owning the offending files.
        let mut groups: Vec<(Option<&TaskNode>, Vec<Finding>)> = Vec::new();
        for finding in open {
            let owner = outcomes
                .iter()
                .find(|o| o.files.contains(&finding.path))
                .and_then(|o| plan.node(&o.task_id));
            match groups
                .iter_mut()
                .find(|(task, _)| task.map(|t| &t.id) == owner.map(|t| &t.id))
            {
                Some((_, list)) => list.push(finding),
                None => groups.push((owner, vec![finding])),
            }
        }
        for (owner, group) in groups {
            let listing = group
                .iter()
                .map(|f| {
                    format!(
                        "- [{:?}] {}:{} — {} Remediation: {}",
                        f.severity, f.path, f.line, f.message, f.remediation
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            let mut instruction = format!(
                "## Your assignment (security fix, round {round})\nFix the blocking security findings below in production code. Tests are locked and must stay green; run them before you finish.\n{listing}\n"
            );
            if let Some(hint) = &supervisor_hint {
                instruction.push_str(&format!(
                    "\n## Guidance from the human supervisor\n{hint}\n"
                ));
            }
            let mut assignment = audit_assignment(
                ctx,
                plan,
                BACKEND_DEV,
                "security_fix",
                TaskKind::Code,
                instruction,
            );
            assignment.attempt = round;
            assignment.task_id = owner.map(|t| t.id.clone());
            if let Some(task) = owner {
                assignment.context["task"] = serde_json::to_value(task)?;
                assignment.context["task_count"] = json!(plan.execution_order.len());
            }
            let locked = ctx.snapshot_tests();
            ctx.run_agent(assignment).await?;
            ctx.restore_locked_tests(&locked, owner.map(|t| t.id.as_str()).unwrap_or("audit"));
            let report = ctx
                .verify_tests(
                    owner.map(|t| t.id.as_str()),
                    "security_fix",
                    round,
                    "pass",
                    &[],
                )
                .await;
            if report.ok {
                ctx.commit(&format!("fix(security): round {round}")).await?;
            } else {
                ctx.sh("git checkout -q -- . && git clean -fdq").await?;
                ctx.emitter.log(
                    "orchestrator",
                    "warn",
                    "correctif de sécurité annulé : il cassait les tests",
                );
            }
        }

        // Re-evaluate: the deterministic scanner always, the LLM auditor only if it had
        // raised blocking findings itself.
        if !blocking(&llm_findings, policy.block_on_severity).is_empty() {
            llm_findings = auditor_findings(&run_auditor(ctx, plan, &[]).await?);
        }
        findings = scan_workspace(ctx.ws())
            .into_iter()
            .chain(llm_findings.clone())
            .collect();
    }

    // Reviewer asked for changes: one revision pass, kept only if the tests stay green.
    if review.verdict.as_deref() == Some("request_changes") {
        revise(
            ctx,
            plan,
            &format!("Code review comments to address:\n{}", review.summary),
            "refactor: address review comments",
        )
        .await?;
        review.confidence = review.confidence.min(0.7);
    }
    Ok(AuditOutcome {
        findings,
        waived,
        review,
    })
}

/// A developer revision pass outside the TDD loop (review comments, supervisor feedback).
/// The change is kept only if the test-suite stays green.
async fn revise(ctx: &RunCtx, plan: &Plan, request: &str, commit_message: &str) -> Result<bool> {
    let instruction = format!(
        "## Goal\n{}\n\n## Your assignment (revision)\n{request}\n\nApply the requested changes to production code. Tests are locked and must stay green; run them before you finish.\n",
        plan.goal
    );
    let locked = ctx.snapshot_tests();
    ctx.run_agent(audit_assignment(
        ctx,
        plan,
        BACKEND_DEV,
        "revision",
        TaskKind::Code,
        instruction,
    ))
    .await?;
    ctx.restore_locked_tests(&locked, "revision");
    let report = ctx.verify_tests(None, "revision", 1, "pass", &[]).await;
    if report.ok {
        ctx.commit(commit_message).await?;
        Ok(true)
    } else {
        ctx.sh("git checkout -q -- . && git clean -fdq").await?;
        ctx.emitter.log(
            "orchestrator",
            "warn",
            "révision annulée : elle cassait les tests",
        );
        Ok(false)
    }
}

fn pull_request_body(
    plan: &Plan,
    outcomes: &[TddOutcome],
    audit: &AuditOutcome,
    tests: &str,
    policy_mode: &str,
    cost: &str,
) -> String {
    let mut body = format!("## Objectif\n\n{}\n\n## Tâches livrées (cycle TDD)\n\n| Tâche | Tentatives GREEN | Refactor | Confiance |\n|---|---|---|---|\n", plan.goal);
    for o in outcomes {
        body.push_str(&format!(
            "| {} — {} | {} | {} | {:.0} % |\n",
            o.task_id,
            o.title.replace('|', "/"),
            o.attempts,
            if o.refactored { "oui" } else { "—" },
            o.confidence * 100.0
        ));
    }
    body.push_str(&format!(
        "\n## Vérifications\n\n- Tests : {tests}\n- Revue de code : {} (confiance {:.0} %)\n",
        audit.review.summary,
        audit.review.confidence * 100.0
    ));
    if audit.findings.is_empty() {
        body.push_str("- Sécurité : aucun finding ouvert\n");
    } else {
        body.push_str(&format!(
            "- Sécurité : {} finding(s) ouvert(s){}\n",
            audit.findings.len(),
            if audit.waived {
                " — **dérogation accordée par le superviseur**"
            } else {
                ""
            }
        ));
        for f in &audit.findings {
            body.push_str(&format!(
                "  - [{:?}] `{}:{}` {} ({})\n",
                f.severity, f.path, f.line, f.message, f.rule_id
            ));
        }
    }
    body.push_str(&format!("\n## Traçabilité\n\n- Supervision : {policy_mode}\n- Coût LLM : {cost}\n- Planificateur : {}\n", plan.planner));
    body
}

/// Final verification, merge gate, and the merge-request artifact.
async fn delivery_stage(
    ctx: &RunCtx,
    plan: &Plan,
    outcomes: &[TddOutcome],
    audit: &AuditOutcome,
    branch: &str,
) -> Result<()> {
    let policy = ctx.policy();
    let mut revision = 0;
    loop {
        // Never deliver on a stale verdict: the full suite runs one last time.
        let tests = ctx
            .verify_tests(None, "delivery", revision + 1, "pass", &[])
            .await;
        if !tests.ok {
            return Err(Error::Aborted(format!(
                "livraison refusée : la suite de tests est rouge ({})",
                tests.summary()
            )));
        }
        ctx.commit("chore: finalise change set").await?;
        let stat = ctx
            .sh(&format!("git --no-pager diff --stat {BASE_BRANCH}..HEAD"))
            .await?
            .stdout;
        let commits = ctx
            .sh(&format!("git --no-pager log --oneline {BASE_BRANCH}..HEAD"))
            .await?
            .stdout;
        let diff = ctx
            .sandbox
            .exec(
                &format!("git --no-pager diff --no-color {BASE_BRANCH}..HEAD"),
                std::time::Duration::from_secs(60),
            )
            .await?
            .stdout;

        let task_confidence =
            outcomes.iter().map(|o| o.confidence).sum::<f64>() / outcomes.len().max(1) as f64;
        let open_blocking = !blocking(&audit.findings, policy.block_on_severity).is_empty();
        let confidence = 0.5 * task_confidence
            + 0.3 * audit.review.confidence
            + 0.2 * if open_blocking { 0.4 } else { 1.0 };
        let title = format!("feat: {}", plan.goal.chars().take(72).collect::<String>());
        // The simulated backend spends nothing: its figure is what the same token volume
        // would cost at the fallback price list, and must be labelled as such.
        let cost = if ctx.router.backend_name() == "simulated" {
            format!(
                "aucun (backend simulé ; équivalent estimé {:.4} $)",
                ctx.budget.spent()
            )
        } else {
            format!("{:.4} $", ctx.budget.spent())
        };
        let mode = serde_json::to_value(policy.default_mode)?
            .as_str()
            .unwrap_or_default()
            .to_uppercase();
        let body = pull_request_body(plan, outcomes, audit, &tests.summary(), &mode, &cost);
        let pull_request = json!({
            "kind": "pull_request",
            "title": title,
            "body": body,
            "branch": branch,
            "base": BASE_BRANCH,
            "commits": commits.lines().collect::<Vec<_>>(),
            "diff_stat": stat,
            "diff": diff.chars().take(400_000).collect::<String>(),
            "confidence": confidence,
        });

        let mut exceptions = Vec::new();
        if open_blocking && !audit.waived {
            exceptions.push(ExceptionSignal::new(
                ExceptionKind::SecurityViolation,
                "findings de sécurité bloquants non résolus",
            ));
        }
        let outcome = ctx
            .checkpoint(Checkpoint {
                stage: Stage::Merge,
                task_id: None,
                title: "Merge request finale".into(),
                summary: format!(
                    "{} — {} commit(s), tests verts.",
                    title,
                    commits.lines().count()
                ),
                proposal: pull_request.clone(),
                confidence,
                exceptions,
                options: vec![
                    GateOption::new(GateDecision::Approve, "Approuver la merge request"),
                    GateOption::new(GateDecision::RequestChanges, "Demander des modifications"),
                    GateOption::new(GateDecision::Reject, "Rejeter la merge request"),
                ],
            })
            .await;
        match outcome {
            GateOutcome::Approved { .. } => {
                ctx.emitter.emit(
                    EventType::ArtifactCreated,
                    "orchestrator",
                    None,
                    pull_request,
                );
                return Ok(());
            }
            GateOutcome::ChangesRequested { feedback } if revision < policy.max_revisions => {
                revision += 1;
                revise(
                    ctx,
                    plan,
                    &format!(
                        "The human supervisor reviewed the merge request and asks:\n{feedback}"
                    ),
                    "refactor: address supervisor feedback",
                )
                .await?;
            }
            GateOutcome::ChangesRequested { .. } => {
                return Err(Error::Aborted(
                    "merge request : nombre maximal de révisions atteint".into(),
                ))
            }
            GateOutcome::Rejected { reason, by_human } => {
                return Err(rejection("merge request", reason, by_human))
            }
        }
    }
}
