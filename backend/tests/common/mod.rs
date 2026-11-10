//! Shared helpers for the integration tests.
//!
//! These tests run real pipelines: scripted "LLM", but a real sandbox (local), real git and
//! real pytest. They need `bash`, `git`, `timeout` and `pytest` on the PATH.
#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;

use omniforge::config::Settings;
use omniforge::events::{Event, EventType};
use omniforge::governance::{GateDecision, GateResolution, SupervisionMode, SupervisionPolicy};
use omniforge::orchestrator::state::{RunState, RunStatus};
use omniforge::orchestrator::{Orchestrator, RunRequest};
use tokio::sync::broadcast::error::RecvError;

pub struct Factory {
    pub orchestrator: Arc<Orchestrator>,
    pub dir: tempfile::TempDir,
}

pub fn factory() -> Factory {
    let dir = tempfile::tempdir().expect("temp dir");
    let orchestrator = Orchestrator::new(Settings::for_tests(dir.path())).expect("orchestrator");
    Factory { orchestrator, dir }
}

pub fn policy(mode: SupervisionMode) -> SupervisionPolicy {
    SupervisionPolicy {
        default_mode: mode,
        ..Default::default()
    }
}

pub fn request(input: &str, supervision: SupervisionPolicy) -> RunRequest {
    RunRequest {
        input: input.to_string(),
        supervision: Some(supervision),
        ..Default::default()
    }
}

pub fn decide(decision: GateDecision, feedback: Option<&str>) -> GateResolution {
    GateResolution {
        decision,
        feedback: feedback.map(str::to_string),
        resolver: "test".into(),
    }
}

/// Follows a run to its end, answering every gate with `on_gate`.
pub async fn drive(
    orchestrator: &Arc<Orchestrator>,
    run_id: &str,
    mut on_gate: impl FnMut(&Event) -> GateResolution,
) -> (RunState, Vec<Event>) {
    let (backlog, mut live) = orchestrator.store().subscribe(run_id, 0);
    let mut events: Vec<Event> = Vec::new();
    let mut pending = backlog;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    'follow: loop {
        for event in pending.drain(..) {
            if events.last().is_some_and(|last| event.seq <= last.seq) {
                continue;
            }
            events.push(event.clone());
            match event.kind {
                EventType::GateRequested => {
                    let gate_id = event.payload["gate_id"].as_str().expect("gate id");
                    orchestrator
                        .resolve_gate(run_id, gate_id, on_gate(&event))
                        .expect("resolve gate");
                }
                EventType::RunStatusChanged => {
                    let status: RunStatus =
                        serde_json::from_value(event.payload["status"].clone()).expect("status");
                    if !status.is_active() {
                        break 'follow;
                    }
                }
                _ => {}
            }
        }
        match tokio::time::timeout_at(deadline, live.recv()).await {
            Ok(Ok(event)) => pending.push(event),
            Ok(Err(RecvError::Lagged(_))) => {
                let last = events.last().map(|e| e.seq).unwrap_or(0);
                pending = orchestrator.store().events(run_id, last);
            }
            Ok(Err(RecvError::Closed)) => break,
            Err(_) => panic!(
                "run {run_id} did not finish in time; last events: {:#?}",
                &events[events.len().saturating_sub(5)..]
            ),
        }
    }
    // A run that did not complete explains itself in the test output: the verdict of its
    // last test execution is what a failing assertion needs.
    if let Some(error) = orchestrator.state(run_id).ok().and_then(|s| s.error) {
        let last_tests = of_kind(&events, EventType::TddTestResult)
            .last()
            .map(|e| {
                e.payload["report"]["output"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string()
            })
            .unwrap_or_else(|| "(no test run)".into());
        eprintln!("run {run_id} ended with: {error}\n--- last test output ---\n{last_tests}\n---");
    }
    // Let the run task finish its bookkeeping (sandbox stop, registry removal).
    for _ in 0..100 {
        if orchestrator
            .state(run_id)
            .map(|s| s.status != RunStatus::Interrupted)
            .unwrap_or(true)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    (orchestrator.state(run_id).expect("state"), events)
}

pub fn of_kind(events: &[Event], kind: EventType) -> Vec<&Event> {
    events.iter().filter(|e| e.kind == kind).collect()
}

pub fn gate_stages(events: &[Event]) -> Vec<String> {
    of_kind(events, EventType::GateRequested)
        .iter()
        .map(|e| e.payload["stage"].as_str().unwrap_or_default().to_string())
        .collect()
}

pub fn has_tool(name: &str) -> bool {
    std::process::Command::new("bash")
        .args(["-c", &format!("command -v {name}")])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub const JWT_TASK: &str = "Créer un endpoint FastAPI sécurisé avec JWT";

pub const PRD: &str = "# Plateforme de réservation de salles\n\nContexte général du projet.\n\n\
## Authentification\nGestion des comptes.\n- Inscription par e-mail\n  - Le mot de passe est haché\n  - Un e-mail en double est refusé\n- Connexion avec jeton JWT\n\n\
## Réservations\n- Modèle de données des salles\n- Création d'une réservation par un utilisateur\n- Tableau de bord des réservations\n\n\
## Paiement\n- Paiement en ligne avec Stripe\n- Webhooks de confirmation\n";
