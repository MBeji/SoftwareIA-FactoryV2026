//! Simulated LLM backend — a deterministic, offline stand-in for real models.
//!
//! It lets the whole factory run without any API key: the orchestration, the sandbox, the
//! TDD loop, the supervision gates and the event stream are all *real*; only the "thinking"
//! is scripted. It is what the test-suite and the dashboard demo run on.
//!
//! Two scripts are available, chosen from the task text:
//! * a realistic one — a standard-library JWT module — whose first implementation forgets
//!   the expiry check (→ self-correction loop), carries an unused import (→ refactor) and
//!   compares signatures with `!=` (→ security audit);
//! * a generic scaffold for any other task.
//!
//! Directives embedded in the request steer exception paths for demos and tests:
//! `[sim:diverge]` (the developer never fixes the failing test) and
//! `[sim:low-confidence]` (the architect reports a low confidence).
//!
//! The backend is stateless: it derives its position in a conversation from the number of
//! assistant turns, and its context from [`CallMeta::context`](super::CallMeta).

use std::sync::LazyLock;
use std::time::Duration;

use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};

use super::{ChatRole, LlmBackend, LlmRequest, LlmResponse, ToolCall};
use crate::error::Result;
use crate::ingestion::planner::heuristic_plan;

pub struct SimulatedBackend {
    delay: Duration,
}

impl SimulatedBackend {
    pub fn new(delay_ms: u64) -> Self {
        Self {
            delay: Duration::from_millis(delay_ms),
        }
    }
}

/// Simulation directives found in a request, e.g. `[sim:diverge]`.
pub fn directives(text: &str) -> Vec<String> {
    static DIRECTIVE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[sim:([a-z-]+)\]").unwrap());
    DIRECTIVE
        .captures_iter(text)
        .map(|c| c[1].to_string())
        .collect()
}

fn has_directive(ctx: &Value, name: &str) -> bool {
    ctx["directives"]
        .as_array()
        .is_some_and(|d| d.iter().any(|v| v.as_str() == Some(name)))
}

/// ASCII, underscore-separated identifier usable as a Python module name.
pub fn slugify(text: &str, max: usize) -> String {
    let mut out = String::new();
    for c in text.to_lowercase().chars() {
        let mapped = match c {
            'à' | 'â' | 'ä' | 'á' | 'ã' => "a",
            'é' | 'è' | 'ê' | 'ë' => "e",
            'î' | 'ï' | 'í' => "i",
            'ô' | 'ö' | 'ó' | 'õ' => "o",
            'ù' | 'û' | 'ü' | 'ú' => "u",
            'ç' => "c",
            'ñ' => "n",
            'œ' => "oe",
            c if c.is_ascii_alphanumeric() => {
                out.push(c);
                continue;
            }
            _ => "_",
        };
        if mapped == "_" && (out.is_empty() || out.ends_with('_')) {
            continue;
        }
        out.push_str(mapped);
    }
    let mut slug: String = out.trim_matches('_').chars().take(max).collect();
    slug = slug.trim_matches('_').to_string();
    if slug.is_empty() || slug.starts_with(|c: char| c.is_ascii_digit()) {
        slug = format!("f_{slug}");
    }
    slug
}

struct Script {
    jwt: bool,
    module: String,
    task_id: String,
    title: String,
    criteria: Vec<String>,
    risk_score: u64,
    diverge: bool,
}

impl Script {
    fn from_context(ctx: &Value) -> Self {
        static JWT: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"(?i)\bjwt\b|json web token").unwrap());
        let task = &ctx["task"];
        let task_id = task["id"].as_str().unwrap_or("T1").to_string();
        let title = task["title"].as_str().unwrap_or("Tâche").to_string();
        let scope = format!(
            "{title} {}",
            task["description"].as_str().unwrap_or_default()
        );
        let jwt = JWT.is_match(&scope);
        let single_task = ctx["task_count"].as_u64().unwrap_or(1) <= 1;
        let module = if jwt && single_task {
            "jwt_auth".to_string()
        } else {
            slugify(&format!("{task_id} {title}"), 48)
        };
        Self {
            jwt,
            module,
            task_id,
            title,
            criteria: task["acceptance_criteria"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|c| c.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            risk_score: task["risk"]["score"].as_u64().unwrap_or(0),
            diverge: has_directive(ctx, "diverge"),
        }
    }

    fn test_path(&self) -> String {
        format!("tests/test_{}.py", self.module)
    }

    fn source_path(&self) -> String {
        format!("app/{}.py", self.module)
    }

    /// Does this attempt still carry the functional bug?
    fn buggy(&self, attempt: u32) -> bool {
        self.diverge || (attempt <= 1 && (self.jwt || self.risk_score >= 9))
    }

    fn tests(&self) -> String {
        if self.jwt {
            JWT_TESTS.replace("__MODULE__", &self.module)
        } else {
            GENERIC_TESTS
                .replace("__DOC_TITLE__", &self.title.replace(['"', '\\'], ""))
                .replace("__MODULE__", &self.module)
                .replace("__ID__", &py(&self.task_id))
                .replace("__TITLE__", &py(&self.title))
                .replace("__CRITERIA__", &py_list(&self.criteria))
        }
    }

    fn implementation(
        &self,
        expiry_check: bool,
        unused_import: bool,
        constant_time: bool,
    ) -> String {
        if self.jwt {
            JWT_IMPL
                .replace("@@IMPORT_OS@@", if unused_import { "import os\n" } else { "" })
                .replace(
                    "@@SIGCHECK@@",
                    if constant_time {
                        "    if not hmac.compare_digest(signature, expected_signature):"
                    } else {
                        "    if signature != expected_signature:"
                    },
                )
                .replace(
                    "@@EXPIRY@@",
                    if expiry_check {
                        "    current = time.time() if now is None else now\n    if claims.get(\"exp\", 0) <= current:\n        raise TokenError(\"token expired\")\n"
                    } else {
                        ""
                    },
                )
        } else {
            GENERIC_IMPL
                .replace("__DOC_TITLE__", &self.title.replace(['"', '\\'], ""))
                .replace("__ID__", &py(&self.task_id))
                .replace("__TITLE__", &py(&self.title))
                .replace("__CRITERIA__", &py_list(&self.criteria))
                .replace("__READY__", if expiry_check { "True" } else { "False" })
        }
    }
}

/// JSON string literals are valid Python string literals.
fn py(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into())
}

fn py_list(items: &[String]) -> String {
    serde_json::to_string(items).unwrap_or_else(|_| "[]".into())
}

const JWT_TESTS: &str = r#""""Tests (RED) — HS256 JSON Web Tokens."""

import base64
import json

import pytest

from app.__MODULE__ import TokenError, create_token, verify_token

SECRET = "unit-test-secret"


def _segment(data: dict) -> str:
    return base64.urlsafe_b64encode(json.dumps(data).encode()).rstrip(b"=").decode()


def test_valid_token_returns_its_claims():
    token = create_token("alice", SECRET, ttl_seconds=60, now=1_000)
    claims = verify_token(token, SECRET, now=1_010)
    assert claims["sub"] == "alice"
    assert claims["exp"] == 1_060


def test_expired_token_is_rejected():
    token = create_token("alice", SECRET, ttl_seconds=60, now=1_000)
    with pytest.raises(TokenError):
        verify_token(token, SECRET, now=1_061)


def test_tampered_payload_is_rejected():
    header, _, signature = create_token("alice", SECRET, now=1_000).split(".")
    forged_payload = create_token("mallory", SECRET, now=1_000).split(".")[1]
    with pytest.raises(TokenError):
        verify_token(f"{header}.{forged_payload}.{signature}", SECRET, now=1_010)


def test_wrong_secret_is_rejected():
    token = create_token("alice", SECRET, now=1_000)
    with pytest.raises(TokenError):
        verify_token(token, "another-secret", now=1_010)


def test_unsigned_alg_none_token_is_rejected():
    token = f"{_segment({'alg': 'none'})}.{_segment({'sub': 'alice', 'exp': 9_999_999_999})}."
    with pytest.raises(TokenError):
        verify_token(token, SECRET, now=1_010)


def test_malformed_token_is_rejected():
    with pytest.raises(TokenError):
        verify_token("not-a-token", SECRET)
"#;

const JWT_IMPL: &str = r#""""Minimal HS256 JSON Web Token helpers (standard library only)."""

import base64
import hashlib
import hmac
import json
@@IMPORT_OS@@import time


class TokenError(Exception):
    """Raised when a token is malformed, forged or expired."""


def _b64encode(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode("ascii")


def _b64decode(data: str) -> bytes:
    return base64.urlsafe_b64decode(data + "=" * (-len(data) % 4))


def _sign(message: str, secret: str) -> str:
    mac = hmac.new(secret.encode("utf-8"), message.encode("utf-8"), hashlib.sha256)
    return _b64encode(mac.digest())


def create_token(subject: str, secret: str, ttl_seconds: int = 900, now: float | None = None) -> str:
    """Return a signed token for `subject`, valid for `ttl_seconds`."""
    issued_at = int(time.time() if now is None else now)
    header = _b64encode(json.dumps({"alg": "HS256", "typ": "JWT"}).encode("utf-8"))
    claims = {"sub": subject, "iat": issued_at, "exp": issued_at + ttl_seconds}
    payload = _b64encode(json.dumps(claims).encode("utf-8"))
    signing_input = f"{header}.{payload}"
    return f"{signing_input}.{_sign(signing_input, secret)}"


def verify_token(token: str, secret: str, now: float | None = None) -> dict:
    """Return the claims of a valid token; raise TokenError otherwise."""
    try:
        header_b64, payload_b64, signature = token.split(".")
        header = json.loads(_b64decode(header_b64))
        claims = json.loads(_b64decode(payload_b64))
    except (ValueError, TypeError) as exc:
        raise TokenError("malformed token") from exc
    if not isinstance(header, dict) or header.get("alg") != "HS256":
        raise TokenError("unsupported algorithm")
    expected_signature = _sign(f"{header_b64}.{payload_b64}", secret)
@@SIGCHECK@@
        raise TokenError("invalid signature")
@@EXPIRY@@    return claims
"#;

const GENERIC_TESTS: &str = r#""""Tests (RED) — __DOC_TITLE__."""

from app import __MODULE__


def test_feature_is_described():
    info = __MODULE__.describe()
    assert info["id"] == __ID__
    assert info["title"] == __TITLE__


def test_acceptance_criteria_are_tracked():
    assert __MODULE__.acceptance_criteria() == __CRITERIA__


def test_feature_is_ready():
    assert __MODULE__.is_ready() is True
"#;

const GENERIC_IMPL: &str = r#""""__DOC_TITLE__ — placeholder generated by the OmniForge simulated backend.

No model is behind the simulated backend: this scaffold only exists so that the whole
workflow (TDD loop, supervision, audit, delivery) can be exercised offline.
"""

_CRITERIA = __CRITERIA__


def describe() -> dict:
    return {"id": __ID__, "title": __TITLE__}


def acceptance_criteria() -> list[str]:
    return list(_CRITERIA)


def is_ready() -> bool:
    return __READY__
"#;

fn architecture_doc(ctx: &Value) -> String {
    let goal = ctx["goal"].as_str().unwrap_or("Objectif du run");
    let mut doc = format!(
        "# Architecture — {goal}\n\n\
         > Document produit par l'agent Architecte (backend simulé).\n\n\
         ## Contexte\n\n{goal}\n\n\
         ## Décisions\n\n\
         | Décision | Justification | Compromis |\n|---|---|---|\n\
         | Paquet Python `app/`, un module par tâche | Frontières nettes, tâches indépendantes et testables isolément | Davantage de petits fichiers |\n\
         | Bibliothèque standard uniquement | La sandbox n'a pas d'accès réseau : aucune dépendance à installer | Fonctions utilitaires à écrire soi-même |\n\
         | Tests `pytest` dans `tests/`, un fichier par module | Cycle TDD rapide, verdict binaire pour l'orchestrateur | — |\n\n\
         ## Composants\n\n| Tâche | Module | Tests | Composant |\n|---|---|---|---|\n"
    );
    let task_count = ctx["tasks"].as_array().map(Vec::len).unwrap_or(0);
    for task in ctx["tasks"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let script = Script::from_context(&json!({ "task": task, "task_count": task_count }));
        doc.push_str(&format!(
            "| {} — {} | `{}` | `{}` | {} |\n",
            script.task_id,
            script.title.replace('|', "/"),
            script.source_path(),
            script.test_path(),
            task["component"].as_str().unwrap_or("backend"),
        ));
    }
    doc.push_str(
        "\n## Stratégie de test\n\n\
         1. **RED** — l'agent QA écrit les tests à partir des critères d'acceptation ; ils doivent échouer.\n\
         2. **GREEN** — le développeur écrit le code minimal ; les tests sont verrouillés.\n\
         3. **REFACTOR** — lint puis correction, tests toujours verts.\n\n\
         ## Risques et points ouverts\n\n\
         - Les dépendances externes (paiement, e-mail…) sont à isoler derrière des interfaces simulables.\n",
    );
    doc
}

fn call(step: usize, index: usize, name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: format!("sim-{step}-{index}"),
        name: name.to_string(),
        arguments,
    }
}

fn finish(step: usize, summary: &str, confidence: f64) -> (Option<String>, Vec<ToolCall>) {
    (
        None,
        vec![call(
            step,
            0,
            "finish",
            json!({ "summary": summary, "confidence": confidence }),
        )],
    )
}

fn say(thought: &str, calls: Vec<ToolCall>) -> (Option<String>, Vec<ToolCall>) {
    (Some(thought.to_string()), calls)
}

fn respond(request: &LlmRequest) -> (Option<String>, Vec<ToolCall>) {
    let ctx = &request.meta.context;
    let step = request
        .messages
        .iter()
        .filter(|m| m.role == ChatRole::Assistant)
        .count();
    let attempt = request.meta.attempt.max(1);
    let script = Script::from_context(ctx);

    match (request.meta.phase.as_str(), step) {
        // ---- Product Owner: structured plan -------------------------------------------
        ("plan", _) => {
            let input = ctx["input"].as_str().unwrap_or_default();
            let level = ctx["level"].as_u64().unwrap_or(1) as u8;
            let capacity = ctx["sprint_capacity"].as_u64().unwrap_or(13) as u32;
            let plan = heuristic_plan(input, level, capacity);
            let body = json!({ "goal": plan.goal, "confidence": plan.confidence, "nodes": plan.nodes });
            (Some(body.to_string()), vec![])
        }

        // ---- Architect ----------------------------------------------------------------
        ("architecture", 0) => say(
            "J'explore le workspace existant avant de décider de l'architecture.",
            vec![call(step, 0, "list_files", json!({}))],
        ),
        ("architecture", 1) => say(
            "Je retiens l'architecture la plus simple qui couvre le plan et je la consigne avec ses compromis.",
            vec![call(step, 0, "write_file", json!({ "path": "docs/ARCHITECTURE.md", "content": architecture_doc(ctx) }))],
        ),
        ("architecture", _) => {
            if has_directive(ctx, "low-confidence") {
                finish(step, "Architecture proposée, mais plusieurs exigences sont ambiguës : je préfère une validation humaine.", 0.45)
            } else {
                finish(step, "Architecture consignée dans docs/ARCHITECTURE.md : un module par tâche, bibliothèque standard, tests pytest.", 0.86)
            }
        }

        // ---- QA: RED ------------------------------------------------------------------
        ("red", 0) => say(
            "Je traduis chaque critère d'acceptation en un test précis. Ces tests doivent échouer tant que le code n'existe pas.",
            vec![call(step, 0, "write_file", json!({ "path": script.test_path(), "content": script.tests() }))],
        ),
        ("red", _) => finish(
            step,
            &format!("Tests écrits dans {} : chemin nominal, cas limites et cas d'erreur.", script.test_path()),
            0.88,
        ),

        // ---- Developer: GREEN ---------------------------------------------------------
        ("green", 0) => say(
            "J'observe d'abord les tests en échec pour comprendre le contrat attendu.",
            vec![call(step, 0, "read_file", json!({ "path": script.test_path() }))],
        ),
        ("green", 1) => {
            let thought = if attempt == 1 {
                "J'écris l'implémentation minimale qui satisfait le contrat décrit par les tests."
            } else {
                "J'applique le diagnostic de la boucle d'auto-correction et je corrige le code de production."
            };
            let content = script.implementation(!script.buggy(attempt), true, false);
            let mut calls = vec![call(step, 0, "write_file", json!({ "path": "app/__init__.py", "content": "" }))];
            calls.push(call(step, 1, "write_file", json!({ "path": script.source_path(), "content": content })));
            if attempt > 1 {
                calls.remove(0);
            }
            say(thought, calls)
        }
        ("green", 2) => say("Je vérifie mon travail en exécutant la suite de tests.", vec![call(step, 0, "run_tests", json!({}))]),
        ("green", _) => {
            if script.buggy(attempt) {
                finish(step, "Implémentation écrite, mais au moins un test échoue encore : un diagnostic est nécessaire.", 0.4)
            } else {
                finish(step, &format!("{} implémenté ; tous les tests passent.", script.source_path()), 0.9)
            }
        }

        // ---- Critic: reflection on a failing run (JSON) -------------------------------
        ("reflect", _) => {
            let body = if script.diverge {
                json!({
                    "root_cause": "Le test de disponibilité échoue toujours : la cause n'est pas identifiable à partir de la trace seule.",
                    "fix_plan": "Demander des précisions sur le comportement attendu ou une indication du superviseur.",
                    "confidence": 0.3,
                })
            } else if script.jwt {
                json!({
                    "root_cause": "verify_token valide la signature mais ne compare jamais la claim `exp` à l'heure courante : un jeton expiré est accepté.",
                    "fix_plan": "Après la vérification de signature, lever TokenError si claims['exp'] <= now (now injectable pour les tests).",
                    "confidence": 0.9,
                })
            } else {
                json!({
                    "root_cause": "is_ready() renvoie False alors que le test attend True une fois la fonctionnalité livrée.",
                    "fix_plan": "Faire renvoyer True par is_ready() ; ne pas toucher aux tests.",
                    "confidence": 0.85,
                })
            };
            (Some(body.to_string()), vec![])
        }

        // ---- Developer: REFACTOR (lint) -----------------------------------------------
        ("refactor", 0) if script.jwt => say(
            "Le linter signale un import inutilisé : je le retire sans changer le comportement.",
            vec![call(step, 0, "write_file", json!({ "path": script.source_path(), "content": script.implementation(true, false, false) }))],
        ),
        ("refactor", _) => finish(step, "Avertissements de lint traités ; aucun changement de comportement.", 0.9),

        // ---- Developer: security fix --------------------------------------------------
        ("security_fix", 0) if script.jwt => say(
            "La comparaison de signature avec `!=` fuit de l'information par le temps de réponse : je passe à hmac.compare_digest.",
            vec![call(step, 0, "write_file", json!({ "path": script.source_path(), "content": script.implementation(true, false, true) }))],
        ),
        ("security_fix", _) => finish(step, "Findings de sécurité bloquants corrigés.", 0.88),

        // ---- Security auditor ---------------------------------------------------------
        ("security_review", 0) => say(
            "Je passe en revue le changement comme un attaquant : entrées non fiables, secrets, cryptographie.",
            vec![call(step, 0, "list_files", json!({}))],
        ),
        ("security_review", _) => (
            None,
            vec![call(step, 0, "finish", json!({
                "summary": "Revue manuelle : pas de finding supplémentaire au-delà du scanner statique.",
                "confidence": 0.8,
                "findings": [],
            }))],
        ),

        // ---- Code reviewer ------------------------------------------------------------
        ("review", 0) => say(
            "Je relis l'historique : un commit de tests, puis le code, puis le refactor — conforme au cycle TDD.",
            vec![call(step, 0, "git", json!({ "args": "log --oneline -12" }))],
        ),
        ("review", _) => (
            None,
            vec![call(step, 0, "finish", json!({
                "summary": "Le changement couvre les critères d'acceptation, les tests sont pertinents et le code est lisible.",
                "confidence": 0.87,
                "verdict": "approve",
            }))],
        ),

        _ => finish(step, "(backend simulé) aucune modification supplémentaire à apporter.", 0.8),
    }
}

#[async_trait]
impl LlmBackend for SimulatedBackend {
    fn name(&self) -> &'static str {
        "simulated"
    }

    async fn complete(&self, model: &str, request: &LlmRequest) -> Result<LlmResponse> {
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        let (content, tool_calls) = respond(request);
        let prompt_chars: usize = request
            .messages
            .iter()
            .map(|m| m.content.as_deref().map(str::len).unwrap_or(0))
            .sum();
        let completion_chars = content.as_deref().map(str::len).unwrap_or(0)
            + tool_calls
                .iter()
                .map(|c| c.arguments.to_string().len())
                .sum::<usize>();
        Ok(LlmResponse {
            content,
            tool_calls,
            model: format!("sim/{model}"),
            prompt_tokens: (prompt_chars / 4) as u64,
            completion_tokens: (completion_chars / 4) as u64,
            cost_usd: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_valid_python_identifiers() {
        assert_eq!(
            slugify("E1-T2 Création d'une réservation", 48),
            "e1_t2_creation_d_une_reservation"
        );
        assert_eq!(slugify("42 façons", 48), "f_42_facons");
        assert_eq!(slugify("???", 48), "f_");
        assert!(slugify(&"x".repeat(200), 48).len() <= 48);
    }

    #[test]
    fn reads_directives() {
        assert_eq!(
            directives("Faire X [sim:diverge] puis [sim:low-confidence]"),
            vec!["diverge", "low-confidence"]
        );
        assert!(directives("rien").is_empty());
    }

    #[test]
    fn jwt_script_fixes_one_defect_per_phase() {
        let ctx = json!({ "task": { "id": "T1", "title": "Créer un endpoint sécurisé avec JWT" }, "task_count": 1 });
        let script = Script::from_context(&ctx);
        assert!(script.jwt);
        assert_eq!(script.source_path(), "app/jwt_auth.py");
        let first = script.implementation(!script.buggy(1), true, false);
        assert!(
            !first.contains("token expired")
                && first.contains("import os")
                && first.contains("signature != expected")
        );
        let second = script.implementation(!script.buggy(2), true, false);
        assert!(second.contains("token expired"));
        let hardened = script.implementation(true, false, true);
        assert!(hardened.contains("compare_digest") && !hardened.contains("import os"));
    }
}
