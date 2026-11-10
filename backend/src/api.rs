//! HTTP API and live event stream (axum).
//!
//! In the reference deployment this service sits behind an API gateway (TLS, SSO, rate
//! limiting). On its own it enforces an optional static API key and a CORS allow-list.

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, Multipart, Path, Query, Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::error::Error;
use crate::events::Event;
use crate::governance::{GateResolution, SupervisionPolicy};
use crate::ingestion::planner::heuristic_plan;
use crate::ingestion::{classify, load_document, normalize_input};
use crate::orchestrator::{Orchestrator, RunRequest};

#[derive(Clone)]
pub struct AppState {
    pub orchestrator: Arc<Orchestrator>,
    pub api_key: Option<String>,
}

pub struct ApiError(Error);

impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        Self(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::Invalid(_) | Error::Json(_) => StatusCode::BAD_REQUEST,
            Error::Conflict(_) => StatusCode::CONFLICT,
            Error::Policy(_) => StatusCode::FORBIDDEN,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!("request failed: {}", self.0);
        }
        (status, Json(json!({ "error": self.0.to_string() }))).into_response()
    }
}

type ApiResult<T> = Result<Json<T>, ApiError>;

/// Builds the application router. `cors_origins` empty = same-origin only; `*` = any.
pub fn router(state: AppState, cors_origins: &[String]) -> Router {
    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::OPTIONS])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION])
        .allow_origin(if cors_origins.iter().any(|o| o == "*") {
            AllowOrigin::any()
        } else {
            AllowOrigin::list(
                cors_origins
                    .iter()
                    .filter_map(|o| HeaderValue::from_str(o).ok()),
            )
        });

    let api = Router::new()
        .route("/meta", get(meta))
        .route("/plan", post(preview_plan))
        .route("/runs", get(list_runs).post(create_run))
        .route(
            "/runs/upload",
            post(upload_run).layer(DefaultBodyLimit::max(25 * 1024 * 1024)),
        )
        .route("/runs/{id}", get(get_run))
        .route("/runs/{id}/events", get(get_events))
        .route("/runs/{id}/stream", get(stream))
        .route("/runs/{id}/gates/{gate_id}/resolve", post(resolve_gate))
        .route("/runs/{id}/supervision", put(set_supervision))
        .route("/runs/{id}/cancel", post(cancel_run))
        .route("/runs/{id}/terminal", post(terminal))
        .route("/runs/{id}/workspace/tree", get(workspace_tree))
        .route("/runs/{id}/workspace/file", get(workspace_file))
        .route("/runs/{id}/workspace/diff", get(workspace_diff))
        .layer(middleware::from_fn_with_state(state.clone(), authenticate));

    Router::new()
        .route("/health", get(health))
        .nest("/api", api)
        .layer(cors)
        .with_state(state)
}

/// Static API key check: `Authorization: Bearer …`. The WebSocket route also accepts
/// `?token=`, because browsers cannot set headers on an upgrade request; it is the only
/// route where a key may travel in the URL.
async fn authenticate(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let Some(expected) = state.api_key.as_deref() else {
        return next.run(request).await;
    };
    let bearer = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string);
    let query_token = request
        .uri()
        .query()
        .filter(|_| request.uri().path().ends_with("/stream"))
        .and_then(|q| {
            q.split('&')
                .find_map(|pair| pair.strip_prefix("token=").map(str::to_string))
        });
    let provided = bearer.or(query_token).unwrap_or_default();
    if constant_time_eq(provided.as_bytes(), expected.as_bytes()) {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing or invalid API key" })),
        )
            .into_response()
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "omniforge", "version": env!("CARGO_PKG_VERSION") }))
}

async fn meta(State(state): State<AppState>) -> Json<Value> {
    Json(state.orchestrator.meta())
}

#[derive(Deserialize)]
struct PlanPreview {
    input: String,
    #[serde(default)]
    level: Option<u8>,
}

/// Dry run of the ingestion engine: classification and deterministic plan, no execution.
async fn preview_plan(
    State(state): State<AppState>,
    Json(body): Json<PlanPreview>,
) -> ApiResult<Value> {
    let input = normalize_input(&body.input)?;
    let classification = classify(&input, None);
    let level = body
        .level
        .filter(|l| (1..=3).contains(l))
        .unwrap_or(classification.level);
    let plan = heuristic_plan(
        &input,
        level,
        state.orchestrator.config().planning.sprint_capacity_points,
    );
    Ok(Json(
        json!({ "classification": classification, "plan": plan }),
    ))
}

async fn list_runs(State(state): State<AppState>) -> Json<Value> {
    Json(json!({ "runs": state.orchestrator.list() }))
}

async fn create_run(
    State(state): State<AppState>,
    Json(body): Json<RunRequest>,
) -> Result<Response, ApiError> {
    let run = state.orchestrator.start_run(body)?;
    Ok((StatusCode::CREATED, Json(run)).into_response())
}

/// Multipart variant for documents: `file` (.md, .txt, .pdf) plus optional `title`,
/// `level` and `supervision` (JSON) fields.
async fn upload_run(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Response, ApiError> {
    let mut request = RunRequest::default();
    let invalid = |e: axum::extract::multipart::MultipartError| {
        ApiError(Error::Invalid(format!("invalid upload: {e}")))
    };
    while let Some(field) = multipart.next_field().await.map_err(invalid)? {
        match field.name().unwrap_or_default() {
            "file" => {
                let filename = field.file_name().unwrap_or("document.txt").to_string();
                let bytes = field.bytes().await.map_err(invalid)?;
                request.input = load_document(&filename, &bytes).await?;
                request.filename = Some(filename);
            }
            "title" => request.title = Some(field.text().await.map_err(invalid)?),
            "level" => request.level = field.text().await.map_err(invalid)?.trim().parse().ok(),
            "supervision" => {
                let raw = field.text().await.map_err(invalid)?;
                request.supervision = Some(serde_json::from_str(&raw).map_err(Error::from)?);
            }
            _ => {}
        }
    }
    if request.input.is_empty() {
        return Err(Error::Invalid("the upload has no `file` field".into()).into());
    }
    let run = state.orchestrator.start_run(request)?;
    Ok((StatusCode::CREATED, Json(run)).into_response())
}

async fn get_run(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    Ok(Json(
        serde_json::to_value(state.orchestrator.state(&id)?).map_err(Error::from)?,
    ))
}

#[derive(Deserialize)]
struct Since {
    #[serde(default)]
    since: u64,
}

async fn get_events(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<Since>,
) -> ApiResult<Value> {
    if !state.orchestrator.store().has_run(&id) {
        return Err(Error::NotFound(format!("run '{id}'")).into());
    }
    Ok(Json(
        json!({ "events": state.orchestrator.store().events(&id, q.since) }),
    ))
}

/// WebSocket: replays the backlog after `since`, then streams live events.
async fn stream(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<Since>,
    upgrade: WebSocketUpgrade,
) -> Response {
    if !state.orchestrator.store().has_run(&id) {
        return ApiError(Error::NotFound(format!("run '{id}'"))).into_response();
    }
    upgrade.on_upgrade(move |socket| pump(socket, state, id, q.since))
}

async fn send_event(socket: &mut WebSocket, event: &Event) -> bool {
    match serde_json::to_string(event) {
        Ok(text) => socket.send(Message::Text(text.into())).await.is_ok(),
        Err(_) => true,
    }
}

async fn pump(mut socket: WebSocket, state: AppState, run_id: String, since: u64) {
    let store = state.orchestrator.store().clone();
    let (backlog, mut live) = store.subscribe(&run_id, since);
    let mut last = since;
    for event in &backlog {
        if !send_event(&mut socket, event).await {
            return;
        }
        last = event.seq;
    }
    loop {
        tokio::select! {
            received = live.recv() => match received {
                Ok(event) => {
                    if event.seq <= last {
                        continue;
                    }
                    if !send_event(&mut socket, &event).await {
                        return;
                    }
                    last = event.seq;
                }
                // Slow consumer: resynchronise from the store instead of dropping events.
                Err(RecvError::Lagged(_)) => {
                    for event in store.events(&run_id, last) {
                        if !send_event(&mut socket, &event).await {
                            return;
                        }
                        last = event.seq;
                    }
                }
                Err(RecvError::Closed) => return,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return,
                Some(Ok(_)) => {}
            },
        }
    }
}

async fn resolve_gate(
    State(state): State<AppState>,
    Path((id, gate_id)): Path<(String, String)>,
    Json(resolution): Json<GateResolution>,
) -> ApiResult<Value> {
    state.orchestrator.resolve_gate(&id, &gate_id, resolution)?;
    Ok(Json(json!({ "ok": true })))
}

async fn set_supervision(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(policy): Json<SupervisionPolicy>,
) -> ApiResult<SupervisionPolicy> {
    Ok(Json(
        state.orchestrator.set_supervision(&id, policy, "user")?,
    ))
}

async fn cancel_run(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    state.orchestrator.cancel(&id, "l'utilisateur").await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct TerminalCommand {
    command: String,
}

async fn terminal(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<TerminalCommand>,
) -> ApiResult<Value> {
    let res = state.orchestrator.terminal(&id, &body.command).await?;
    Ok(Json(serde_json::to_value(res).map_err(Error::from)?))
}

async fn workspace_tree(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    Ok(Json(
        json!({ "files": state.orchestrator.workspace(&id)?.list() }),
    ))
}

#[derive(Deserialize)]
struct FileQuery {
    path: String,
}

async fn workspace_file(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<FileQuery>,
) -> ApiResult<Value> {
    let content = state.orchestrator.workspace(&id)?.read(&q.path)?;
    Ok(Json(json!({ "path": q.path, "content": content })))
}

async fn workspace_diff(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    Ok(Json(json!({ "diff": state.orchestrator.diff(&id).await? })))
}
