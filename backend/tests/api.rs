//! HTTP API tests: a real server on an ephemeral port, driven with an HTTP client.

mod common;

use std::time::Duration;

use common::*;
use omniforge::api::{router, AppState};
use serde_json::{json, Value};

struct Server {
    base: String,
    http: reqwest::Client,
    _factory: Factory,
}

async fn serve(api_key: Option<&str>) -> Server {
    let factory = factory();
    let app = router(
        AppState::new(factory.orchestrator.clone(), api_key.map(str::to_string)),
        &["http://localhost:3000".to_string()],
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Server {
        base,
        http: reqwest::Client::new(),
        _factory: factory,
    }
}

impl Server {
    async fn get(&self, path: &str) -> (u16, Value) {
        let res = self
            .http
            .get(format!("{}{path}", self.base))
            .send()
            .await
            .unwrap();
        (
            res.status().as_u16(),
            res.json().await.unwrap_or(Value::Null),
        )
    }

    async fn send(&self, method: reqwest::Method, path: &str, body: Value) -> (u16, Value) {
        let res = self
            .http
            .request(method, format!("{}{path}", self.base))
            .json(&body)
            .send()
            .await
            .unwrap();
        (
            res.status().as_u16(),
            res.json().await.unwrap_or(Value::Null),
        )
    }

    async fn post(&self, path: &str, body: Value) -> (u16, Value) {
        self.send(reqwest::Method::POST, path, body).await
    }

    async fn wait_for(&self, run_id: &str, statuses: &[&str]) -> Value {
        for _ in 0..600 {
            let (_, state) = self.get(&format!("/api/runs/{run_id}")).await;
            if statuses.contains(&state["status"].as_str().unwrap_or_default()) {
                return state;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("run {run_id} never reached {statuses:?}");
    }
}

#[tokio::test]
async fn a_run_can_be_created_followed_and_inspected_over_http() {
    let s = serve(None).await;
    assert_eq!(s.get("/health").await.1["status"], "ok");

    let (status, run) = s
        .post(
            "/api/runs",
            json!({ "input": JWT_TASK, "supervision": { "default_mode": "autonomous" } }),
        )
        .await;
    assert_eq!(status, 201);
    let id = run["id"].as_str().unwrap();
    let state = s.wait_for(id, &["completed", "failed", "cancelled"]).await;
    assert_eq!(state["status"], "completed", "{}", state["error"]);
    assert_eq!(state["tasks"]["T1"]["status"], "done");
    assert!(state["metrics"]["llm"]["calls"].as_u64().unwrap() > 10);

    // Incremental event polling (the WebSocket fallback).
    let (_, all) = s.get(&format!("/api/runs/{id}/events")).await;
    let total = all["events"].as_array().unwrap().len();
    let (_, tail) = s
        .get(&format!("/api/runs/{id}/events?since={}", total - 3))
        .await;
    assert_eq!(tail["events"].as_array().unwrap().len(), 3);
    assert_eq!(all["events"][0]["type"], "run.created");

    // Workspace: tree, file, cumulative diff — and no way out of it.
    let (_, tree) = s.get(&format!("/api/runs/{id}/workspace/tree")).await;
    let paths: Vec<&str> = tree["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["path"].as_str())
        .collect();
    assert!(
        paths.contains(&"app/jwt_auth.py")
            && paths.contains(&"tests/test_jwt_auth.py")
            && paths.contains(&"docs/ARCHITECTURE.md")
    );
    let (_, file) = s
        .get(&format!(
            "/api/runs/{id}/workspace/file?path=app/jwt_auth.py"
        ))
        .await;
    assert!(file["content"]
        .as_str()
        .unwrap()
        .contains("def verify_token"));
    let (_, diff) = s.get(&format!("/api/runs/{id}/workspace/diff")).await;
    assert!(diff["diff"]
        .as_str()
        .unwrap()
        .contains("+++ b/tests/test_jwt_auth.py"));
    assert_eq!(
        s.get(&format!(
            "/api/runs/{id}/workspace/file?path=../../events.db"
        ))
        .await
        .0,
        403
    );
    assert_eq!(
        s.get(&format!("/api/runs/{id}/workspace/file?path=/etc/passwd"))
            .await
            .0,
        403
    );
    assert_eq!(
        s.get(&format!("/api/runs/{id}/workspace/file?path=nope.py"))
            .await
            .0,
        404
    );

    let (_, list) = s.get("/api/runs").await;
    assert_eq!(list["runs"][0]["id"], id);
    assert_eq!(
        (
            list["runs"][0]["tasks_done"].as_u64(),
            list["runs"][0]["mode"].as_str()
        ),
        (Some(1), Some("autonomous"))
    );
    // A finished run no longer accepts live operations.
    assert_eq!(
        s.post(
            &format!("/api/runs/{id}/terminal"),
            json!({ "command": "ls" })
        )
        .await
        .0,
        409
    );
    assert_eq!(
        s.post(&format!("/api/runs/{id}/cancel"), json!({})).await.0,
        409
    );
}

#[tokio::test]
async fn gates_supervision_and_terminal_are_operable_over_http() {
    let s = serve(None).await;
    let (_, run) = s
        .post(
            "/api/runs",
            json!({ "input": JWT_TASK, "supervision": { "default_mode": "hitl" } }),
        )
        .await;
    let id = run["id"].as_str().unwrap();
    let state = s.wait_for(id, &["waiting_approval"]).await;
    let gate = &state["gates"][0];
    assert_eq!(gate["request"]["stage"], "plan");
    assert_eq!(
        gate["request"]["proposal"]["plan"]["execution_order"],
        json!(["T1"])
    );
    assert_eq!(gate["request"]["options"].as_array().unwrap().len(), 3);

    // Interactive terminal while the run is parked; dangerous commands are refused.
    let (status, out) = s
        .post(
            &format!("/api/runs/{id}/terminal"),
            json!({ "command": "echo bonjour" }),
        )
        .await;
    assert_eq!((status, out["stdout"].as_str()), (200, Some("bonjour\n")));
    assert_eq!(
        s.post(
            &format!("/api/runs/{id}/terminal"),
            json!({ "command": "sudo reboot" })
        )
        .await
        .0,
        403
    );

    // Live switch to autonomous, then approve the pending gate: nothing else will ask.
    let (status, policy) = s
        .send(
            reqwest::Method::PUT,
            &format!("/api/runs/{id}/supervision"),
            json!({ "default_mode": "autonomous" }),
        )
        .await;
    assert_eq!(
        (status, policy["default_mode"].as_str()),
        (200, Some("autonomous"))
    );
    assert_eq!(
        s.send(
            reqwest::Method::PUT,
            &format!("/api/runs/{id}/supervision"),
            json!({ "confidence_threshold": 7 })
        )
        .await
        .0,
        400
    );
    let gate_id = gate["gate_id"].as_str().unwrap();
    assert_eq!(
        s.post(
            &format!("/api/runs/{id}/gates/{gate_id}/resolve"),
            json!({ "decision": "approve", "feedback": "ok" })
        )
        .await
        .0,
        200
    );
    assert_eq!(
        s.post(
            &format!("/api/runs/{id}/gates/{gate_id}/resolve"),
            json!({ "decision": "approve" })
        )
        .await
        .0,
        404
    );

    let done = s.wait_for(id, &["completed", "failed", "cancelled"]).await;
    assert_eq!(done["status"], "completed", "{}", done["error"]);
    assert_eq!(done["gates"].as_array().unwrap().len(), 1);
    assert_eq!(done["supervision"]["default_mode"], "autonomous");
    assert_eq!(done["gates"][0]["resolution"]["feedback"], "ok");
}

#[tokio::test]
async fn documents_can_be_uploaded_and_plans_previewed() {
    let s = serve(None).await;
    // Dry run of the ingestion engine.
    let (status, preview) = s.post("/api/plan", json!({ "input": PRD })).await;
    assert_eq!(status, 200);
    assert_eq!(preview["classification"]["level"], 3);
    assert_eq!(
        preview["plan"]["execution_order"].as_array().unwrap().len(),
        7
    );
    assert_eq!(s.post("/api/plan", json!({ "input": "  " })).await.0, 400);

    // Multipart upload of a Markdown specification, with a supervision policy.
    let form = reqwest::multipart::Form::new()
        .part(
            "file",
            reqwest::multipart::Part::bytes(PRD.as_bytes().to_vec())
                .file_name("cahier-des-charges.md"),
        )
        .text("title", "Réservation de salles")
        .text("supervision", r#"{"default_mode":"hitl"}"#);
    let res = s
        .http
        .post(format!("{}/api/runs/upload", s.base))
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 201);
    let run: Value = res.json().await.unwrap();
    assert_eq!(
        (run["title"].as_str(), run["filename"].as_str()),
        (Some("Réservation de salles"), Some("cahier-des-charges.md"))
    );
    let id = run["id"].as_str().unwrap();
    let parked = s.wait_for(id, &["waiting_approval"]).await;
    assert_eq!(parked["classification"]["level"], 3);
    assert_eq!(
        parked["plan"]["sprints"].as_array().map(|x| x.len() >= 2),
        Some(true)
    );
    assert_eq!(
        s.post(&format!("/api/runs/{id}/cancel"), json!({})).await.0,
        200
    );
    assert_eq!(s.wait_for(id, &["cancelled"]).await["status"], "cancelled");

    // Unsupported formats and malformed bodies are client errors.
    let bad = reqwest::multipart::Form::new().part(
        "file",
        reqwest::multipart::Part::bytes(vec![1, 2, 3]).file_name("spec.docx"),
    );
    assert_eq!(
        s.http
            .post(format!("{}/api/runs/upload", s.base))
            .multipart(bad)
            .send()
            .await
            .unwrap()
            .status()
            .as_u16(),
        400
    );
    assert_eq!(s.get("/api/runs/run-unknown").await.0, 404);
    let malformed = s
        .http
        .post(format!("{}/api/runs", s.base))
        .header("content-type", "application/json")
        .body("{")
        .send()
        .await
        .unwrap();
    assert!(malformed.status().is_client_error());
}

#[tokio::test]
async fn pdf_specifications_are_ingested_when_pdftotext_is_available() {
    if !has_tool("pdftotext") {
        eprintln!("skipped: pdftotext not installed");
        return;
    }
    // A minimal single-page PDF whose text is a one-line request.
    let stream = "BT /F1 12 Tf 72 720 Td (Ajouter un export CSV des rapports) Tj ET";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
        format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let mut pdf = String::from("%PDF-1.4\n");
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.push_str(&format!("{} 0 obj\n{object}\nendobj\n", i + 1));
    }
    let xref = pdf.len();
    pdf.push_str(&format!(
        "xref\n0 {}\n0000000000 65535 f \n",
        objects.len() + 1
    ));
    for offset in offsets {
        pdf.push_str(&format!("{offset:010} 00000 n \n"));
    }
    pdf.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    ));

    let s = serve(None).await;
    let form = reqwest::multipart::Form::new()
        .part(
            "file",
            reqwest::multipart::Part::bytes(pdf.into_bytes()).file_name("demande.pdf"),
        )
        .text("supervision", r#"{"default_mode":"hitl"}"#);
    let res = s
        .http
        .post(format!("{}/api/runs/upload", s.base))
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 201);
    let run: Value = res.json().await.unwrap();
    assert_eq!(run["input_excerpt"], "Ajouter un export CSV des rapports");
    s.post(
        &format!("/api/runs/{}/cancel", run["id"].as_str().unwrap()),
        json!({}),
    )
    .await;
}

#[tokio::test]
async fn the_api_key_protects_everything_but_the_health_probe() {
    let s = serve(Some("s3cret-key")).await;
    assert_eq!(s.get("/health").await.0, 200);
    assert_eq!(s.get("/api/runs").await.0, 401);
    assert_eq!(s.get("/api/meta").await.0, 401);
    assert_eq!(
        s.post("/api/runs", json!({ "input": JWT_TASK })).await.0,
        401
    );

    let authed = |path: &str, key: &str| {
        s.http
            .get(format!("{}{path}", s.base))
            .bearer_auth(key)
            .send()
    };
    assert_eq!(
        authed("/api/runs", "wrong")
            .await
            .unwrap()
            .status()
            .as_u16(),
        401
    );
    assert_eq!(
        authed("/api/runs", "s3cret-key")
            .await
            .unwrap()
            .status()
            .as_u16(),
        200
    );
    // Browsers cannot set headers on WebSocket upgrades: the stream route — and only it —
    // accepts the key as `?token=`.
    assert_eq!(s.get("/api/meta?token=s3cret-key").await.0, 401);
    assert_eq!(s.get("/api/runs/run-x/stream?token=nope").await.0, 401);
    // With the right key the request gets past authentication; it is then refused only
    // because a plain GET is not a WebSocket upgrade.
    let upgraded = s.get("/api/runs/run-x/stream?token=s3cret-key").await.0;
    assert!(
        upgraded != 401 && (400..500).contains(&upgraded),
        "unexpected status {upgraded}"
    );

    let meta: Value = authed("/api/meta", "s3cret-key")
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(meta["roles"].as_array().unwrap().len(), 7);
    assert_eq!(meta["llm_backend"], "simulated");
    assert_eq!(meta["routing"]["routes"]["test_generation"], "critic");
}

#[tokio::test]
async fn cross_site_pages_cannot_drive_the_api() {
    let s = serve(None).await;
    let post = |origin: Option<&'static str>| {
        let mut request = s
            .http
            .post(format!("{}/api/runs", s.base))
            .json(&json!({ "input": JWT_TASK, "supervision": { "default_mode": "hitl" } }));
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        request.send()
    };
    // A page on another site: refused, and nothing is started.
    assert_eq!(
        post(Some("https://evil.example"))
            .await
            .unwrap()
            .status()
            .as_u16(),
        403
    );
    // The multipart upload is a "simple" request that CORS does not preflight.
    let form = reqwest::multipart::Form::new().part(
        "file",
        reqwest::multipart::Part::bytes(PRD.as_bytes().to_vec()).file_name("prd.md"),
    );
    let upload = s
        .http
        .post(format!("{}/api/runs/upload", s.base))
        .header("origin", "https://evil.example")
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(upload.status().as_u16(), 403);
    assert_eq!(s.get("/api/runs").await.1["runs"], json!([]));

    // The dashboard's own origin, and non-browser clients (no Origin header), are served.
    let allowed = post(Some("http://localhost:3000")).await.unwrap();
    assert_eq!(allowed.status().as_u16(), 201);
    let run: Value = allowed.json().await.unwrap();
    let id = run["id"].as_str().unwrap();
    assert_eq!(post(None).await.unwrap().status().as_u16(), 201);

    // WebSocket handshakes are not covered by CORS either: a foreign page must not be
    // able to read a run's event stream.
    let stream = |origin: &'static str| {
        s.http
            .get(format!("{}/api/runs/{id}/stream", s.base))
            .header("origin", origin)
            .send()
    };
    assert_eq!(
        stream("https://evil.example")
            .await
            .unwrap()
            .status()
            .as_u16(),
        403
    );
    assert_ne!(
        stream("http://localhost:3000")
            .await
            .unwrap()
            .status()
            .as_u16(),
        403
    );
}

#[tokio::test]
async fn concurrent_runs_are_capped() {
    let s = serve(None).await;
    let start = || {
        s.post(
            "/api/runs",
            json!({ "input": JWT_TASK, "supervision": { "default_mode": "hitl" } }),
        )
    };
    let mut ids = Vec::new();
    for _ in 0..4 {
        let (status, run) = start().await;
        assert_eq!(status, 201);
        ids.push(run["id"].as_str().unwrap().to_string());
    }
    // Each HITL run is parked on its first gate and holds a sandbox: the fifth is refused.
    let (status, body) = start().await;
    assert_eq!(status, 409, "{body}");
    assert!(body["error"].as_str().unwrap().contains("limit 4"));
    // Freeing a slot makes room again.
    assert_eq!(
        s.post(&format!("/api/runs/{}/cancel", ids[0]), json!({}))
            .await
            .0,
        200
    );
    assert_eq!(start().await.0, 201);
}
