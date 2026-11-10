//! `omniforge` — command-line entry point.
//!
//! * `omniforge serve` — HTTP + WebSocket API for the Mission Control dashboard.
//! * `omniforge run "…"` — run a request headless and follow it in the terminal.
//! * `omniforge plan "…"` — classify a request and print its task network (dry run).

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use omniforge::api::{self, AppState};
use omniforge::config::{FactoryConfig, Settings};
use omniforge::events::{Event, EventType};
use omniforge::governance::{GateDecision, GateResolution, SupervisionMode};
use omniforge::ingestion::planner::heuristic_plan;
use omniforge::ingestion::{classify, load_document};
use omniforge::orchestrator::state::RunStatus;
use omniforge::orchestrator::{Orchestrator, RunRequest};
use tokio::sync::broadcast::error::RecvError;

#[derive(Parser)]
#[command(
    name = "omniforge",
    version,
    about = "OmniForge AI — agentic software factory"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum Mode {
    Hitl,
    Hotl,
    Autonomous,
}

#[derive(Subcommand)]
enum Command {
    /// Start the HTTP + WebSocket API.
    Serve {
        #[command(flatten)]
        settings: Settings,
        #[arg(long, env = "OMNIFORGE_BIND", default_value = "127.0.0.1:8080")]
        bind: String,
        /// Static API key required from clients (recommended as soon as the API is exposed).
        #[arg(long, env = "OMNIFORGE_API_KEY", hide_env_values = true)]
        api_key: Option<String>,
        /// Allowed browser origins, comma-separated. `*` allows any origin.
        #[arg(
            long,
            env = "OMNIFORGE_CORS_ORIGINS",
            value_delimiter = ',',
            default_value = "http://localhost:3000,http://127.0.0.1:3000"
        )]
        cors_origins: Vec<String>,
    },
    /// Run a request headless and stream its events to the terminal.
    Run {
        #[command(flatten)]
        settings: Settings,
        /// The request (user story, epic…). Use --file for a document.
        input: Option<String>,
        /// Specification file (.md, .txt, .pdf).
        #[arg(long)]
        file: Option<PathBuf>,
        /// Supervision mode.
        #[arg(long, value_enum, default_value = "hotl")]
        mode: Mode,
        /// Force the granularity level (1 micro-task, 2 epic, 3 PRD).
        #[arg(long)]
        level: Option<u8>,
        /// Approve every gate automatically instead of prompting.
        #[arg(long)]
        yes: bool,
    },
    /// Classify a request and print its hierarchical task network (no execution).
    Plan {
        input: Option<String>,
        #[arg(long)]
        file: Option<PathBuf>,
        /// Print the full plan as JSON.
        #[arg(long)]
        json: bool,
    },
}

type AnyResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

async fn read_input(
    input: Option<String>,
    file: Option<PathBuf>,
) -> AnyResult<(String, Option<String>)> {
    match (input, file) {
        (_, Some(path)) => {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let bytes = tokio::fs::read(&path).await?;
            Ok((load_document(&name, &bytes).await?, Some(name)))
        }
        (Some(text), None) => Ok((text, None)),
        (None, None) => Err("provide a request as argument or a document with --file".into()),
    }
}

fn first_line(text: &str, max: usize) -> String {
    let line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    if line.chars().count() > max {
        format!("{}…", line.chars().take(max).collect::<String>())
    } else {
        line.to_string()
    }
}

/// One human-readable line (or block) per event.
fn render(event: &Event) -> Option<String> {
    let p = &event.payload;
    let text = |key: &str| p[key].as_str().unwrap_or_default().to_string();
    let task = event
        .task_id
        .as_deref()
        .map(|t| format!("[{t}] "))
        .unwrap_or_default();
    Some(match event.kind {
        EventType::RunPhaseChanged => format!("\n━━━ {} ━━━", text("phase").to_uppercase()),
        EventType::InputClassified => format!(
            "niveau {} — {} ({})",
            p["level"],
            text("label"),
            text("rationale")
        ),
        EventType::PlanCreated => {
            let plan = &p["plan"];
            format!(
                "plan : {} tâche(s), {} point(s), {} sprint(s), confiance {:.0} %",
                plan["execution_order"]
                    .as_array()
                    .map(Vec::len)
                    .unwrap_or(0),
                plan["total_points"],
                plan["sprints"].as_array().map(Vec::len).unwrap_or(0),
                plan["confidence"].as_f64().unwrap_or(0.0) * 100.0
            )
        }
        EventType::AgentTurnStarted => format!("▶ {task}{} — {}", text("name"), text("phase")),
        EventType::AgentThought => format!("    ∴ {}", first_line(&text("text"), 160)),
        EventType::Action => {
            let args = &p["args"];
            let hint = args["path"]
                .as_str()
                .or(args["command"].as_str())
                .or(args["args"].as_str())
                .or(args["pattern"].as_str())
                .unwrap_or("");
            format!("    → {} {}", text("tool"), first_line(hint, 100))
        }
        EventType::Observation => format!(
            "    ← {} {}",
            if p["ok"].as_bool() == Some(true) {
                "ok "
            } else {
                "ERR"
            },
            first_line(&text("output"), 120)
        ),
        EventType::AgentTurnEnded if p["ok"].as_bool() == Some(true) => {
            format!(
                "■ {task}{} (confiance {:.0} %)",
                first_line(&text("summary"), 140),
                p["confidence"].as_f64().unwrap_or(0.0) * 100.0
            )
        }
        EventType::AgentTurnEnded => format!("■ {task}échec : {}", text("error")),
        EventType::TddPhaseChanged => format!(
            "  ◆ {task}TDD {} (tentative {}/{})",
            text("phase").to_uppercase(),
            p["attempt"],
            p["max_attempts"]
        ),
        EventType::TddTestResult => format!(
            "  ✦ {task}tests [{} attendu : {}] {} {}",
            text("phase"),
            text("expected"),
            text("summary"),
            if p["as_expected"].as_bool() == Some(true) {
                "✓"
            } else {
                "✗"
            }
        ),
        EventType::TddReflection => format!(
            "  ↻ {task}diagnostic : {}",
            first_line(&text("root_cause"), 160)
        ),
        EventType::GateEvaluated => {
            let e = &p["evaluation"];
            format!(
                "  ⚖ {} [{}] → {} — {}",
                text("title"),
                e["mode"].as_str().unwrap_or(""),
                e["verdict"].as_str().unwrap_or(""),
                e["reason"].as_str().unwrap_or("")
            )
        }
        EventType::GateRequested => format!(
            "\n⏸  VALIDATION REQUISE — {}\n   {}\n   motif : {}",
            text("title"),
            first_line(&text("summary"), 200),
            text("reason")
        ),
        EventType::GateResolved => {
            format!("⏵  décision : {} ({})", text("decision"), text("resolver"))
        }
        EventType::PolicyViolation => format!(
            "  ⚠ violation de politique [{}] {}",
            text("rule"),
            text("detail")
        ),
        EventType::SecurityFindings => format!(
            "  ⛨ sécurité (tour {}) : {} finding(s), dont {} bloquant(s)",
            p["round"],
            p["findings"].as_array().map(Vec::len).unwrap_or(0),
            p["blocking"]
        ),
        EventType::ArtifactCreated if p["kind"] == "pull_request" => {
            format!(
                "\n┌─ MERGE REQUEST ─ {}\n│ branche {} → {}\n{}",
                text("title"),
                text("branch"),
                text("base"),
                text("body")
                    .lines()
                    .map(|l| format!("│ {l}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        }
        EventType::SupervisionChanged => "  ⚙ politique de supervision mise à jour".to_string(),
        EventType::Log => format!("  · {}", text("message")),
        EventType::RunStatusChanged => match p["error"].as_str() {
            Some(error) => format!("\n● statut : {} — {error}", text("status")),
            None => format!("● statut : {}", text("status")),
        },
        _ => return None,
    })
}

/// Asks the operator to decide on a gate (interactive HITL / HOTL from a terminal).
async fn prompt_gate(event: &Event) -> GateResolution {
    let options = event.payload["options"].clone();
    tokio::task::spawn_blocking(move || {
        for option in options.as_array().into_iter().flatten() {
            println!(
                "   [{}] {}",
                option["decision"].as_str().unwrap_or(""),
                option["label"].as_str().unwrap_or("")
            );
        }
        let ask = |question: &str| -> String {
            print!("{question}");
            let _ = std::io::stdout().flush();
            let mut line = String::new();
            let _ = std::io::stdin().read_line(&mut line);
            line.trim().to_string()
        };
        let decision = loop {
            match ask("   décision — (a)pprouver / (m)odifier / (r)ejeter : ")
                .to_lowercase()
                .as_str()
            {
                "a" | "approve" | "approuver" | "o" | "oui" | "y" => break GateDecision::Approve,
                "m" | "modifier" | "request_changes" | "c" => break GateDecision::RequestChanges,
                "r" | "reject" | "rejeter" | "n" | "non" => break GateDecision::Reject,
                _ => println!("   réponse non comprise"),
            }
        };
        let feedback = Some(ask("   commentaire (optionnel) : ")).filter(|f| !f.is_empty());
        GateResolution {
            decision,
            feedback,
            resolver: "cli".into(),
        }
    })
    .await
    .unwrap_or(GateResolution {
        decision: GateDecision::Reject,
        feedback: None,
        resolver: "cli".into(),
    })
}

async fn run_headless(
    settings: Settings,
    input: String,
    filename: Option<String>,
    mode: Mode,
    level: Option<u8>,
    yes: bool,
) -> AnyResult<bool> {
    let data_dir = settings.data_dir.clone();
    let orchestrator = Orchestrator::new(settings)?;
    let mut supervision = orchestrator.config().supervision.clone();
    supervision.default_mode = match mode {
        Mode::Hitl => SupervisionMode::Hitl,
        Mode::Hotl => SupervisionMode::Hotl,
        Mode::Autonomous => SupervisionMode::Autonomous,
    };
    let run = orchestrator.start_run(RunRequest {
        title: None,
        input,
        filename,
        supervision: Some(supervision),
        level,
    })?;
    println!(
        "run {} — backend LLM : {}, sandbox : {}",
        run.id, run.llm_backend, run.sandbox
    );

    let (backlog, mut live) = orchestrator.store().subscribe(&run.id, 0);
    let mut last = 0;
    let mut pending: Vec<Event> = backlog;
    let status = 'follow: loop {
        for event in pending.drain(..) {
            if event.seq <= last {
                continue;
            }
            last = event.seq;
            if let Some(line) = render(&event) {
                println!("{line}");
            }
            match event.kind {
                EventType::GateRequested => {
                    let resolution = if yes {
                        GateResolution {
                            decision: GateDecision::Approve,
                            feedback: None,
                            resolver: "cli --yes".into(),
                        }
                    } else {
                        prompt_gate(&event).await
                    };
                    let gate_id = event.payload["gate_id"].as_str().unwrap_or_default();
                    orchestrator.resolve_gate(&run.id, gate_id, resolution)?;
                }
                EventType::RunStatusChanged => {
                    let status: Option<RunStatus> =
                        serde_json::from_value(event.payload["status"].clone()).ok();
                    if let Some(status) = status.filter(|s| !s.is_active()) {
                        break 'follow status;
                    }
                }
                _ => {}
            }
        }
        match live.recv().await {
            Ok(event) => pending.push(event),
            Err(RecvError::Lagged(_)) => pending = orchestrator.store().events(&run.id, last),
            Err(RecvError::Closed) => break RunStatus::Interrupted,
        }
    };

    let state = orchestrator.state(&run.id)?;
    println!(
        "\n{} appel(s) LLM, {} token(s), {:.4} $ — workspace : {}",
        state.metrics.llm.calls,
        state.metrics.llm.prompt_tokens + state.metrics.llm.completion_tokens,
        state.metrics.llm.cost_usd,
        data_dir
            .join("runs")
            .join(&run.id)
            .join("workspace")
            .display()
    );
    Ok(status == RunStatus::Completed)
}

async fn serve(
    settings: Settings,
    bind: String,
    api_key: Option<String>,
    cors_origins: Vec<String>,
) -> AnyResult<()> {
    let orchestrator = Orchestrator::new(settings)?;
    let meta = orchestrator.meta();
    let api_key = api_key.filter(|k| !k.is_empty());
    if api_key.is_none() {
        tracing::warn!("no API key configured (OMNIFORGE_API_KEY): the API is open to anyone who can reach {bind}");
    }
    if meta["sandbox"] == "local" {
        tracing::warn!("sandbox=local: agent commands run on this host WITHOUT isolation — demos and tests only");
    }
    let app = api::router(AppState::new(orchestrator, api_key), &cors_origins);
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(
        "OmniForge API listening on http://{bind} (llm: {}, sandbox: {})",
        meta["llm_backend"],
        meta["sandbox"]
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "omniforge=info,tower_http=warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let outcome: AnyResult<bool> = match Cli::parse().command {
        Command::Serve {
            settings,
            bind,
            api_key,
            cors_origins,
        } => serve(settings, bind, api_key, cors_origins)
            .await
            .map(|_| true),
        Command::Run {
            settings,
            input,
            file,
            mode,
            level,
            yes,
        } => match read_input(input, file).await {
            Ok((text, filename)) => run_headless(settings, text, filename, mode, level, yes).await,
            Err(e) => Err(e),
        },
        Command::Plan { input, file, json } => {
            match read_input(input, file).await {
                Ok((text, filename)) => {
                    let classification = classify(&text, filename.as_deref());
                    let capacity = FactoryConfig::default().planning.sprint_capacity_points;
                    let plan = heuristic_plan(&text, classification.level, capacity);
                    if json {
                        println!("{}", serde_json::to_string_pretty(&serde_json::json!({ "classification": classification, "plan": plan })).unwrap_or_default());
                    } else {
                        println!(
                            "Niveau {} — {} ({})\n",
                            classification.level, classification.label, classification.rationale
                        );
                        println!("{}", plan.outline());
                        println!("Ordre d'exécution : {}", plan.execution_order.join(" → "));
                        println!("Chemin critique   : {}", plan.critical_path.join(" → "));
                        for sprint in &plan.sprints {
                            println!(
                                "{} ({} pts, risque max {}) : {}",
                                sprint.name,
                                sprint.points,
                                sprint.max_risk_score,
                                sprint.task_ids.join(", ")
                            );
                        }
                    }
                    Ok(true)
                }
                Err(e) => Err(e),
            }
        }
    };

    match outcome {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}
