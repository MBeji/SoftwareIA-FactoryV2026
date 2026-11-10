//! Input ingestion engine: loads a request of any granularity and classifies it.
//!
//! * Level 1 — micro-task / simple user story
//! * Level 2 — functional epic
//! * Level 3 — full specification / PRD (Markdown, text or PDF)
//!
//! Classification is deterministic (cheap, explainable, overridable): it only decides which
//! planning workflow runs next — the *Routing* pattern applied to the input itself.

pub mod htn;
pub mod planner;

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Hard cap on ingested text, to keep prompts and events bounded.
pub const MAX_INPUT_CHARS: usize = 200_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Signals {
    pub words: usize,
    pub headings: usize,
    pub bullets: usize,
    /// Distinct deliverables enumerated in the sentence ("X, Y et Z").
    pub enumerated_items: usize,
    pub is_document: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Classification {
    pub level: u8,
    pub label: String,
    pub rationale: String,
    pub signals: Signals,
}

static HEADING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^[ \t]{0,3}#{1,6}[ \t]+\S").unwrap());
static BULLET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^[ \t]*(?:[-*+•]|\d+[.)])[ \t]+\S").unwrap());
static PRD_HINT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(cahier des charges|prd|product requirements|sp[ée]cifications? (fonctionnelles?|techniques?)|exigences? (non[- ])?fonctionnelles?)\b").unwrap()
});
static EPIC_HINT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(epic|module|sous-syst[èe]me|parcours|workflow complet|int[ée]gration)\b")
        .unwrap()
});

/// Splits "… avec A, B et C" into its enumerated deliverables.
pub fn enumerated_items(text: &str) -> Vec<String> {
    static SPLIT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)\s*(?:,|;|\bet\b|\band\b|\+|&)\s*").unwrap());
    static LEAD: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\b(?:avec|with|incluant|including|comprenant|:)\s+").unwrap()
    });
    let first_line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    let Some(m) = LEAD.find(first_line) else {
        return vec![];
    };
    SPLIT
        .split(first_line[m.end()..].trim_end_matches(['.', '!']))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Deterministic three-level classifier.
pub fn classify(text: &str, filename: Option<&str>) -> Classification {
    let is_document = filename.is_some_and(|f| {
        let f = f.to_ascii_lowercase();
        f.ends_with(".pdf") || f.ends_with(".md") || f.ends_with(".markdown") || f.ends_with(".txt")
    });
    let signals = Signals {
        words: text.split_whitespace().count(),
        headings: HEADING.find_iter(text).count(),
        bullets: BULLET.find_iter(text).count(),
        enumerated_items: enumerated_items(text).len(),
        is_document,
    };

    let (level, rationale) = if signals.headings >= 3 && signals.bullets >= 4 {
        (
            3,
            format!(
                "document structuré : {} sections et {} exigences listées",
                signals.headings, signals.bullets
            ),
        )
    } else if signals.words >= 400 {
        (
            3,
            format!(
                "texte long ({} mots) : traité comme un cahier des charges",
                signals.words
            ),
        )
    } else if PRD_HINT.is_match(text) && (signals.bullets >= 3 || signals.words >= 120) {
        (
            3,
            "le texte se présente comme un cahier des charges / PRD".to_string(),
        )
    } else if signals.enumerated_items >= 2 {
        (
            2,
            format!(
                "{} livrables énumérés dans la demande",
                signals.enumerated_items
            ),
        )
    } else if signals.bullets >= 3 {
        (
            2,
            format!(
                "{} exigences listées sans structure de document",
                signals.bullets
            ),
        )
    } else if signals.words >= 60 || (EPIC_HINT.is_match(text) && signals.words >= 12) {
        (
            2,
            "demande couvrant un périmètre fonctionnel (epic)".to_string(),
        )
    } else {
        (1, "demande courte et atomique".to_string())
    };

    let label = match level {
        1 => "Micro-tâche / user story",
        2 => "Epic fonctionnelle",
        _ => "Cahier des charges / PRD",
    };
    Classification {
        level,
        label: label.to_string(),
        rationale,
        signals,
    }
}

/// Decodes an uploaded document into text. PDFs go through `pdftotext` (poppler).
pub async fn load_document(filename: &str, bytes: &[u8]) -> Result<String> {
    let lower = filename.to_ascii_lowercase();
    let text = if lower.ends_with(".pdf") {
        pdf_to_text(bytes).await?
    } else if lower.ends_with(".md") || lower.ends_with(".markdown") || lower.ends_with(".txt") {
        String::from_utf8_lossy(bytes).into_owned()
    } else {
        return Err(Error::Invalid(format!(
            "unsupported document type '{filename}' (expected .md, .txt or .pdf)"
        )));
    };
    normalize_input(&text)
}

/// Trims, bounds and validates raw input text.
pub fn normalize_input(text: &str) -> Result<String> {
    let cleaned = text.replace("\r\n", "\n").replace('\u{c}', "\n");
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        return Err(Error::Invalid("the request is empty".into()));
    }
    Ok(cleaned.chars().take(MAX_INPUT_CHARS).collect())
}

async fn pdf_to_text(bytes: &[u8]) -> Result<String> {
    let dir = std::env::temp_dir().join(format!("omniforge-pdf-{}", uuid::Uuid::new_v4().simple()));
    tokio::fs::create_dir_all(&dir).await?;
    let pdf = dir.join("input.pdf");
    tokio::fs::write(&pdf, bytes).await?;
    let result = run_pdftotext(&pdf).await;
    let _ = tokio::fs::remove_dir_all(&dir).await;
    result
}

async fn run_pdftotext(pdf: &Path) -> Result<String> {
    let output = tokio::process::Command::new("pdftotext")
        .args(["-layout", "-enc", "UTF-8"])
        .arg(pdf)
        .arg("-")
        .output()
        .await
        .map_err(|e| {
            Error::Invalid(format!(
                "PDF ingestion needs `pdftotext` (poppler-utils) on the server: {e}"
            ))
        })?;
    if !output.status.success() {
        return Err(Error::Invalid(format!(
            "cannot extract text from the PDF: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    if text.trim().is_empty() {
        return Err(Error::Invalid(
            "the PDF contains no extractable text (scanned document?)".into(),
        ));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_the_three_reference_inputs() {
        let micro = classify("Créer un endpoint FastAPI sécurisé avec JWT", None);
        assert_eq!(micro.level, 1, "{micro:?}");

        let epic = classify(
            "Implémenter le module de facturation avec Stripe, webhooks et dashboard",
            None,
        );
        assert_eq!(epic.level, 2, "{epic:?}");
        assert_eq!(epic.signals.enumerated_items, 3);

        let prd = "# Plateforme de réservation\n\n## Authentification\n- Inscription par e-mail\n- Connexion JWT\n\n\
                   ## Catalogue\n- Recherche de salles\n- Filtres par capacité\n\n## Paiement\n- Paiement Stripe\n- Factures PDF\n";
        let full = classify(prd, Some("prd.md"));
        assert_eq!(full.level, 3, "{full:?}");
        assert!(full.signals.is_document);
    }

    #[test]
    fn enumerations_are_split_on_commas_and_conjunctions() {
        assert_eq!(
            enumerated_items(
                "Implémenter le module de facturation avec Stripe, webhooks et dashboard"
            ),
            vec!["Stripe", "webhooks", "dashboard"]
        );
        assert!(enumerated_items("Corriger le bug d'arrondi").is_empty());
    }

    #[test]
    fn rejects_empty_input() {
        assert!(normalize_input("  \n ").is_err());
        assert_eq!(normalize_input(" a\r\nb ").unwrap(), "a\nb");
    }

    #[tokio::test]
    async fn rejects_unsupported_documents() {
        assert!(load_document("spec.docx", b"x").await.is_err());
        assert_eq!(
            load_document("spec.md", b"# Titre\n").await.unwrap(),
            "# Titre"
        );
    }
}
