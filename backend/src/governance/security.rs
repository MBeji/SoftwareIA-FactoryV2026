//! Deterministic static security scanner (SAST-lite).
//!
//! The LLM security auditor gives a narrative assessment, but blocking decisions rely on
//! these reproducible rules: the workspace is parsed on the host, never executed.

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use super::Severity;
use crate::sandbox::{is_test_path, Workspace};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub rule_id: String,
    pub severity: Severity,
    pub path: String,
    pub line: usize,
    pub message: String,
    pub remediation: String,
    pub snippet: String,
}

struct Rule {
    id: &'static str,
    severity: Severity,
    /// File extensions the rule applies to; empty = every text file.
    exts: &'static [&'static str],
    pattern: Regex,
    /// Lines matching this are ignored (safe idioms, placeholders…).
    unless: Option<Regex>,
    skip_tests: bool,
    message: &'static str,
    remediation: &'static str,
}

const PY: &[&str] = &["py"];
const JS: &[&str] = &["js", "jsx", "ts", "tsx", "mjs", "cjs"];
const CODE: &[&str] = &[
    "py", "js", "jsx", "ts", "tsx", "mjs", "cjs", "go", "rb", "php", "java", "rs",
];

#[allow(clippy::too_many_arguments)]
fn rule(
    id: &'static str,
    severity: Severity,
    exts: &'static [&'static str],
    pattern: &str,
    unless: Option<&str>,
    skip_tests: bool,
    message: &'static str,
    remediation: &'static str,
) -> Rule {
    Rule {
        id,
        severity,
        exts,
        pattern: Regex::new(pattern).expect("valid rule pattern"),
        unless: unless.map(|u| Regex::new(u).expect("valid rule exclusion")),
        skip_tests,
        message,
        remediation,
    }
}

static RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    vec![
        rule(
            "SEC001",
            Severity::High,
            &[],
            r#"(?i)\b\w*(api[_-]?key|secret|passwd|password|private[_-]?key|access[_-]?token)\w*\s*[:=]\s*["'][A-Za-z0-9+/_\-\.]{12,}["']"#,
            Some(r"(?i)environ|getenv|process\.env|example|changeme|placeholder|dummy|<[a-z_-]+>"),
            true,
            "Secret potentiellement codé en dur",
            "Charger le secret depuis l'environnement ou un coffre (Vault, KMS) ; ne jamais le versionner.",
        ),
        rule(
            "SEC002",
            Severity::Critical,
            &[],
            r"-----BEGIN (RSA |EC |DSA |OPENSSH )?PRIVATE KEY-----",
            None,
            false,
            "Clé privée présente dans le dépôt",
            "Révoquer la clé, la retirer de l'historique et la stocker dans un gestionnaire de secrets.",
        ),
        rule(
            "PY001",
            Severity::High,
            PY,
            r"(^|[^.\w])(eval|exec)\s*\(",
            None,
            true,
            "Exécution dynamique de code (eval/exec)",
            "Remplacer par un parseur dédié (ast.literal_eval, json) ou une table de dispatch.",
        ),
        rule(
            "PY002",
            Severity::High,
            PY,
            r"shell\s*=\s*True|os\.system\s*\(|os\.popen\s*\(",
            None,
            true,
            "Commande shell construite dynamiquement (injection de commandes)",
            "Utiliser subprocess.run([...], shell=False) avec une liste d'arguments.",
        ),
        rule(
            "PY003",
            Severity::High,
            PY,
            r"pickle\.loads?\s*\(|yaml\.load\s*\(",
            Some(r"SafeLoader|safe_load"),
            true,
            "Désérialisation non sûre (pickle / yaml.load)",
            "Utiliser json ou yaml.safe_load ; ne jamais désérialiser une entrée non fiable.",
        ),
        rule(
            "PY004",
            Severity::Medium,
            PY,
            r"hashlib\.(md5|sha1)\s*\(",
            Some(r"usedforsecurity\s*=\s*False"),
            true,
            "Fonction de hachage faible (MD5/SHA-1)",
            "Utiliser SHA-256 ou supérieur ; pour des mots de passe, argon2/bcrypt/scrypt.",
        ),
        rule(
            "CRY001",
            Severity::High,
            CODE,
            r"(?i)\b\w*(signature|digest|hmac|password_hash|token_hash|secret)\w*\s*(==|!=)=?\s*[A-Za-z_]",
            Some(r"compare_digest|timingSafeEqual|constant_time|(==|!=)=?\s*(None|True|False|null|undefined|nil)\b"),
            true,
            "Comparaison de secret/signature non résistante aux attaques temporelles",
            "Comparer en temps constant (hmac.compare_digest, crypto.timingSafeEqual).",
        ),
        rule(
            "NET001",
            Severity::High,
            CODE,
            r"verify\s*=\s*False|rejectUnauthorized\s*:\s*false|InsecureSkipVerify\s*:\s*true",
            None,
            true,
            "Vérification TLS désactivée",
            "Conserver la vérification des certificats ; fournir le bundle d'AC si nécessaire.",
        ),
        rule(
            "JWT001",
            Severity::Critical,
            CODE,
            r#"(?i)algorithms?\s*[:=]\s*\[?\s*["']none["']|verify_signature["']?\s*:\s*False"#,
            None,
            true,
            "Validation de signature JWT désactivée (alg=none)",
            "Imposer une liste blanche d'algorithmes (HS256/RS256) et toujours vérifier la signature.",
        ),
        rule(
            "SQL001",
            Severity::High,
            CODE,
            r#"(?i)\.(execute|query|raw)\s*\(\s*(f["']|["'][^"']*["']\s*(%|\+|\.format))"#,
            None,
            true,
            "Requête SQL construite par concaténation/formatage (injection SQL)",
            "Utiliser des requêtes paramétrées ou l'ORM.",
        ),
        rule(
            "JS001",
            Severity::High,
            JS,
            r"(^|[^.\w])eval\s*\(|new\s+Function\s*\(",
            None,
            true,
            "Exécution dynamique de code (eval / new Function)",
            "Supprimer l'évaluation dynamique ; parser les données avec JSON.parse.",
        ),
        rule(
            "JS002",
            Severity::Medium,
            JS,
            r"dangerouslySetInnerHTML|\.innerHTML\s*=",
            None,
            true,
            "Injection HTML brute (risque XSS)",
            "Échapper le contenu ou l'assainir (DOMPurify) avant insertion.",
        ),
    ]
});

/// Scans a single file's content. Exposed for unit tests and the audit tool.
pub fn scan_text(path: &str, content: &str) -> Vec<Finding> {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let in_tests = is_test_path(path);
    let mut findings = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        if line.len() > 2000 {
            continue; // minified / generated content
        }
        for rule in RULES.iter() {
            if rule.skip_tests && in_tests {
                continue;
            }
            if !rule.exts.is_empty() && !rule.exts.contains(&ext.as_str()) {
                continue;
            }
            if !rule.pattern.is_match(line) {
                continue;
            }
            if rule.unless.as_ref().is_some_and(|u| u.is_match(line)) {
                continue;
            }
            findings.push(Finding {
                rule_id: rule.id.to_string(),
                severity: rule.severity,
                path: path.to_string(),
                line: idx + 1,
                message: rule.message.to_string(),
                remediation: rule.remediation.to_string(),
                snippet: line.trim().chars().take(160).collect(),
            });
        }
    }
    findings
}

/// Scans every text file of the workspace, most severe findings first.
pub fn scan_workspace(ws: &Workspace) -> Vec<Finding> {
    let mut findings = Vec::new();
    for entry in ws.list() {
        if entry.size > 512 * 1024 {
            continue;
        }
        if let Ok(content) = ws.read(&entry.path) {
            findings.extend(scan_text(&entry.path, &content));
        }
    }
    findings.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then(a.path.cmp(&b.path))
            .then(a.line.cmp(&b.line))
    });
    findings
}

pub fn blocking(findings: &[Finding], threshold: Severity) -> Vec<Finding> {
    findings
        .iter()
        .filter(|f| f.severity >= threshold)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(path: &str, src: &str) -> Vec<String> {
        scan_text(path, src)
            .into_iter()
            .map(|f| f.rule_id)
            .collect()
    }

    #[test]
    fn flags_timing_unsafe_signature_comparison() {
        assert_eq!(
            ids("app/auth.py", "    if signature != expected_signature:\n"),
            vec!["CRY001"]
        );
        assert!(ids(
            "app/auth.py",
            "    if not hmac.compare_digest(signature, expected_signature):\n"
        )
        .is_empty());
        assert!(ids("app/auth.py", "    if header.get(\"alg\") != \"HS256\":\n").is_empty());
        assert!(ids("app/auth.py", "    if signature == None:\n").is_empty());
    }

    #[test]
    fn flags_hardcoded_secrets_outside_tests() {
        let src = "API_KEY = \"0123456789abcdefghijklmnop\"\n";
        assert_eq!(ids("app/settings.py", src), vec!["SEC001"]);
        assert!(ids("tests/test_settings.py", src).is_empty());
        assert!(ids("app/settings.py", "API_KEY = os.environ[\"API_KEY\"]\n").is_empty());
    }

    #[test]
    fn flags_dangerous_python_idioms() {
        assert_eq!(ids("a.py", "result = eval(user_input)\n"), vec!["PY001"]);
        assert_eq!(
            ids("a.py", "subprocess.run(cmd, shell=True)\n"),
            vec!["PY002"]
        );
        assert_eq!(ids("a.py", "data = yaml.load(stream)\n"), vec!["PY003"]);
        assert!(ids("a.py", "data = yaml.load(stream, Loader=yaml.SafeLoader)\n").is_empty());
        assert!(ids("a.py", "model.eval()\n").is_empty());
        assert_eq!(
            ids("a.py", "requests.get(url, verify=False)\n"),
            vec!["NET001"]
        );
        assert_eq!(
            ids("a.py", "cur.execute(f\"SELECT * FROM t WHERE id={x}\")\n"),
            vec!["SQL001"]
        );
    }

    #[test]
    fn rules_are_scoped_by_language() {
        assert!(ids("README.md", "call eval(x) for fun\n").is_empty());
        assert_eq!(ids("web/app.tsx", "el.innerHTML = html\n"), vec!["JS002"]);
        assert_eq!(
            ids("cfg.py", "jwt.decode(t, algorithms=[\"none\"])\n"),
            vec!["JWT001"]
        );
    }
}
