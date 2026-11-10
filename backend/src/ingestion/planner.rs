//! Planners: turn a classified request into a Hierarchical Task Network.
//!
//! * [`heuristic_plan`] — deterministic decomposition from the document structure. It is
//!   the offline planner and the safety net when a model returns an unusable plan.
//! * [`planning_messages`] / [`parse_llm_plan`] — the Product Owner agent's structured
//!   planning call. The model proposes nodes, dependencies and risk factors; scheduling is
//!   always recomputed by [`Plan::finalize`].

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use super::enumerated_items;
use super::htn::{Component, NodeKind, Plan, Risk, TaskNode};
use crate::error::{Error, Result};
use crate::llm::{extract_json, ChatMessage};

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("valid planner pattern")
}

struct RiskFactor {
    pattern: Regex,
    label: &'static str,
    probability: u8,
    impact: u8,
    mitigation: &'static str,
}

static RISK_FACTORS: LazyLock<Vec<RiskFactor>> = LazyLock::new(|| {
    let f = |pattern: &str, label, probability, impact, mitigation| RiskFactor {
        pattern: re(pattern),
        label,
        probability,
        impact,
        mitigation,
    };
    vec![
        f(
            r"(?i)paiement|payment|stripe|factur|billing|invoice|abonnement|subscription",
            "flux financiers / conformité",
            1,
            2,
            "tests d'intégration sur le bac à sable du prestataire, idempotence des opérations",
        ),
        f(
            r"(?i)\bauth|jwt|oauth|sso|mot de passe|password|permission|rbac|s[ée]curi|chiffr|encrypt|token",
            "sécurité / contrôle d'accès",
            1,
            2,
            "revue de sécurité dédiée et tests négatifs (jetons expirés ou falsifiés)",
        ),
        f(
            r"(?i)webhook|api externe|external api|\btiers\b|third[- ]party|int[ée]gration|\bsms\b|e-?mail",
            "dépendance à un service externe",
            1,
            1,
            "contrat d'interface simulé (mocks), reprise sur erreur",
        ),
        f(
            r"(?i)migration|legacy|reprise de donn[ée]es|import de donn[ée]es",
            "migration de données",
            1,
            2,
            "répétition sur un jeu de données représentatif, plan de retour arrière",
        ),
        f(
            r"(?i)temps r[ée]el|real[- ]?time|websocket|streaming|concurren",
            "temps réel / concurrence",
            2,
            1,
            "tests de charge ciblés, gestion explicite des accès concurrents",
        ),
        f(
            r"(?i)performance|scalab|mont[ée]e en charge|latence|\bcache\b",
            "performance",
            1,
            1,
            "budget de performance mesuré en intégration continue",
        ),
        f(
            r"(?i)rgpd|gdpr|donn[ée]es personnelles|personal data|\bpii\b",
            "données personnelles (RGPD)",
            0,
            2,
            "minimisation des données et journalisation des accès",
        ),
    ]
});

static FRONTEND: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?i)\b(ui|ihm|interface|[ée]cran|page|dashboard|tableau de bord|formulaire|front(end)?|react|next\.?js|composant|responsive|css)\b",
    )
});
static INFRA: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?i)\b(docker|kubernetes|k8s|ci/cd|pipeline|d[ée]ploiement|deploy(ment)?|terraform|monitoring|observabilit[ée]|infrastructure)\b",
    )
});
static DOCS: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)\b(documentation|readme|guide utilisateur|tutoriel)\b"));
static FOUNDATION: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?i)\b(mod[èe]les?|sch[ée]ma|base de donn[ée]es|database|authentification|auth|inscription|connexion|login|configuration|setup|socle|initialis\w*)\b",
    )
});
static FOUNDATION_EPIC: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?i)\b(authentification|auth|s[ée]curit[ée]|socle|fondations?|comptes?( utilisateurs?)?|utilisateurs)\b",
    )
});
static USER_SCOPED: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?i)\b(utilisateurs?|users?|comptes?|accounts?|profils?|admin\w*|r[ôo]les?|membres?|clients?)\b",
    )
});
static REQUIREMENT: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)\b(doit|doivent|devra|devront|permet|permettre|must|shall|should|needs? to)\b")
});

/// Keyword-based risk estimate, used offline and to fill gaps in model answers.
pub fn assess_risk(text: &str) -> Risk {
    let (mut probability, mut impact) = (1u8, 2u8);
    let (mut factors, mut mitigations) = (Vec::new(), Vec::new());
    for factor in RISK_FACTORS.iter().filter(|f| f.pattern.is_match(text)) {
        probability += factor.probability;
        impact += factor.impact;
        factors.push(factor.label.to_string());
        mitigations.push(factor.mitigation);
    }
    Risk::new(probability, impact, factors, mitigations.join(" ; "))
}

pub fn detect_component(text: &str) -> Component {
    if FRONTEND.is_match(text) {
        Component::Frontend
    } else if INFRA.is_match(text) {
        Component::Infra
    } else if DOCS.is_match(text) {
        Component::Docs
    } else {
        Component::Backend
    }
}

/// Fibonacci-like estimate from the number of criteria and the risk score.
pub fn estimate_points(criteria: usize, risk: &Risk) -> u32 {
    let raw = 1 + criteria.min(3) as u32 + u32::from(risk.score) / 6;
    [1, 2, 3, 5, 8].into_iter().find(|p| *p >= raw).unwrap_or(8)
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn short(text: &str, max: usize) -> String {
    let line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    let line = line.trim_start_matches('#').trim();
    if line.chars().count() <= max {
        line.to_string()
    } else {
        format!(
            "{}…",
            line.chars().take(max - 1).collect::<String>().trim_end()
        )
    }
}

fn make_task(
    id: String,
    parent: &str,
    title: &str,
    description: &str,
    mut criteria: Vec<String>,
) -> TaskNode {
    let title = capitalize(title.trim().trim_end_matches(['.', ';', ':']));
    if criteria.is_empty() {
        criteria.push(format!(
            "« {title} » est implémenté et vérifié par des tests automatisés"
        ));
    }
    // Risk is assessed on the task's own wording: the surrounding context (epic, section)
    // would otherwise give every sibling the same score.
    let risk = assess_risk(&format!("{title} {}", criteria.join(" ")));
    TaskNode {
        id,
        title: title.clone(),
        description: description.trim().to_string(),
        kind: NodeKind::Task,
        parent: Some(parent.to_string()),
        estimate_points: estimate_points(criteria.len(), &risk),
        acceptance_criteria: criteria,
        depends_on: vec![],
        component: detect_component(&title),
        risk,
    }
}

fn composite(
    id: &str,
    kind: NodeKind,
    parent: Option<&str>,
    title: &str,
    description: &str,
) -> TaskNode {
    TaskNode {
        id: id.to_string(),
        title: title.to_string(),
        description: description.trim().to_string(),
        kind,
        parent: parent.map(str::to_string),
        acceptance_criteria: vec![],
        depends_on: vec![],
        component: Component::Backend,
        estimate_points: 0,
        risk: Risk::default(),
    }
}

fn empty_plan(
    level: u8,
    goal: String,
    nodes: Vec<TaskNode>,
    confidence: f64,
    planner: &str,
) -> Plan {
    Plan {
        level,
        goal,
        nodes,
        execution_order: vec![],
        waves: vec![],
        critical_path: vec![],
        sprints: vec![],
        total_points: 0,
        confidence,
        planner: planner.to_string(),
        warnings: vec![],
    }
}

struct Section {
    title: String,
    description: String,
    /// (bullet text, nested bullets = acceptance criteria)
    items: Vec<(String, Vec<String>)>,
}

/// Parses headings and bullets of a Markdown / extracted-PDF document.
fn parse_sections(text: &str) -> (Option<String>, Vec<Section>) {
    static MD_HEADING: LazyLock<Regex> =
        LazyLock::new(|| re(r"^[ \t]{0,3}(#{1,6})[ \t]+(.+?)[ \t#]*$"));
    static NUM_HEADING: LazyLock<Regex> =
        LazyLock::new(|| re(r"^[ \t]*\d+(?:\.\d+)*[.)]?[ \t]+(\S.{2,90})$"));
    static BULLET: LazyLock<Regex> =
        LazyLock::new(|| re(r"^([ \t]*)(?:[-*+•]|\d+[.)])[ \t]+(.+)$"));
    static PLAIN_BULLET: LazyLock<Regex> = LazyLock::new(|| re(r"(?m)^[ \t]*[-*+•][ \t]+\S"));

    let has_md_headings = text.lines().any(|l| MD_HEADING.is_match(l));
    // In documents without Markdown headings (PDF exports), numbered lines are headings
    // as long as plain bullets carry the requirements.
    let numbered_are_headings = !has_md_headings && PLAIN_BULLET.is_match(text);

    let mut title = None;
    let mut sections: Vec<Section> = Vec::new();
    for line in text.lines() {
        let heading = if let Some(c) = MD_HEADING.captures(line) {
            Some((c[1].len(), c[2].trim().to_string()))
        } else if numbered_are_headings {
            NUM_HEADING
                .captures(line)
                .map(|c| (2, c[1].trim().to_string()))
        } else {
            None
        };
        if let Some((depth, text)) = heading {
            if depth == 1 && title.is_none() && sections.is_empty() {
                title = Some(text);
            } else {
                sections.push(Section {
                    title: text,
                    description: String::new(),
                    items: vec![],
                });
            }
            continue;
        }
        let Some(section) = sections.last_mut() else {
            continue;
        };
        if let Some(c) = BULLET.captures(line) {
            let indent = c[1].replace('\t', "    ").len();
            let item = c[2].trim().to_string();
            match section.items.last_mut() {
                Some(last) if indent >= 2 => last.1.push(item),
                _ => section.items.push((item, vec![])),
            }
        } else if !line.trim().is_empty()
            && section.items.is_empty()
            && section.description.len() < 400
        {
            if !section.description.is_empty() {
                section.description.push(' ');
            }
            section.description.push_str(line.trim());
        }
    }
    (title, sections)
}

/// Deterministic HTN decomposition.
pub fn heuristic_plan(text: &str, level: u8, sprint_capacity: u32) -> Plan {
    let mut plan = match level {
        1 => plan_micro_task(text),
        2 => plan_epic(text),
        _ => plan_document(text),
    };
    plan.finalize(sprint_capacity);
    let penalty = 0.05 * plan.warnings.len() as f64;
    plan.confidence = (plan.confidence - penalty).clamp(0.3, 0.95);
    plan
}

fn plan_micro_task(text: &str) -> Plan {
    let title = short(text, 90);
    let criteria: Vec<String> = parse_bullets(text);
    let nodes = vec![
        composite("G", NodeKind::Goal, None, &title, text),
        make_task("T1".into(), "G", &title, text, criteria),
    ];
    empty_plan(1, title, nodes, 0.9, "heuristic")
}

fn parse_bullets(text: &str) -> Vec<String> {
    static BULLET: LazyLock<Regex> =
        LazyLock::new(|| re(r"(?m)^[ \t]*(?:[-*+•]|\d+[.)])[ \t]+(.+)$"));
    BULLET
        .captures_iter(text)
        .map(|c| c[1].trim().to_string())
        .collect()
}

fn plan_epic(text: &str) -> Plan {
    let title = short(text, 90);
    let mut nodes = vec![
        composite("G", NodeKind::Goal, None, &title, text),
        composite("E1", NodeKind::Epic, Some("G"), &title, text),
    ];
    let items = enumerated_items(text);
    let bullets = parse_bullets(text);
    let mut confidence = 0.78;

    if items.len() >= 2 {
        // "Implémenter X avec A, B et C" → a foundation task for X, then one task per item.
        static LEAD: LazyLock<Regex> =
            LazyLock::new(|| re(r"(?i)\s+\b(?:avec|with|incluant|including|comprenant)\b|\s*:\s"));
        let first_line = text
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim();
        let head = LEAD
            .find(first_line)
            .map(|m| &first_line[..m.start()])
            .unwrap_or(first_line);
        let mut socle = make_task("T1".into(), "E1", &format!("Socle : {head}"), text, vec![]);
        socle.acceptance_criteria = vec![format!(
            "Le socle « {head} » expose les modèles et services nécessaires aux autres tâches"
        )];
        nodes.push(socle);
        for (i, item) in items.iter().enumerate() {
            let mut task = make_task(
                format!("T{}", i + 2),
                "E1",
                item,
                &format!("Dans le cadre de : {head}"),
                vec![],
            );
            task.depends_on = vec!["T1".into()];
            nodes.push(task);
        }
    } else if bullets.len() >= 2 {
        for (i, bullet) in bullets.iter().enumerate() {
            nodes.push(make_task(format!("T{}", i + 1), "E1", bullet, "", vec![]));
        }
        link_foundations(&mut nodes, "E1");
    } else {
        let sentences: Vec<&str> = text
            .split(['.', '\n', ';'])
            .map(str::trim)
            .filter(|s| s.split_whitespace().count() >= 4)
            .take(8)
            .collect();
        if sentences.len() >= 2 {
            for (i, sentence) in sentences.iter().enumerate() {
                nodes.push(make_task(
                    format!("T{}", i + 1),
                    "E1",
                    &short(sentence, 90),
                    sentence,
                    vec![],
                ));
            }
            link_foundations(&mut nodes, "E1");
            confidence = 0.68;
        } else {
            nodes.push(make_task("T1".into(), "E1", &title, text, vec![]));
            confidence = 0.65;
        }
    }
    empty_plan(2, title, nodes, confidence, "heuristic")
}

/// Inside an epic, non-foundation tasks depend on the foundation ones (models, auth, setup).
fn link_foundations(nodes: &mut [TaskNode], epic_id: &str) {
    let foundations: Vec<String> = nodes
        .iter()
        .filter(|n| {
            n.kind == NodeKind::Task
                && n.parent.as_deref() == Some(epic_id)
                && FOUNDATION.is_match(&n.title)
        })
        .map(|n| n.id.clone())
        .collect();
    if foundations.is_empty() {
        return;
    }
    for node in nodes
        .iter_mut()
        .filter(|n| n.kind == NodeKind::Task && n.parent.as_deref() == Some(epic_id))
    {
        if !foundations.contains(&node.id) {
            node.depends_on.extend(foundations.iter().cloned());
        }
    }
}

fn plan_document(text: &str) -> Plan {
    let (doc_title, sections) = parse_sections(text);
    let goal = doc_title.unwrap_or_else(|| short(text, 90));
    let mut nodes = vec![composite(
        "G",
        NodeKind::Goal,
        None,
        &goal,
        &short(text, 300),
    )];
    let mut confidence = 0.8;

    let mut epic_index = 0;
    for section in sections.iter().filter(|s| !s.items.is_empty()) {
        epic_index += 1;
        let epic_id = format!("E{epic_index}");
        nodes.push(composite(
            &epic_id,
            NodeKind::Epic,
            Some("G"),
            &section.title,
            &section.description,
        ));
        for (i, (item, criteria)) in section.items.iter().enumerate() {
            nodes.push(make_task(
                format!("{epic_id}-T{}", i + 1),
                &epic_id,
                item,
                &section.description,
                criteria.clone(),
            ));
        }
        link_foundations(&mut nodes, &epic_id);
    }

    if epic_index == 0 {
        // Unstructured prose: fall back to requirement sentences.
        confidence = 0.6;
        nodes.push(composite(
            "E1",
            NodeKind::Epic,
            Some("G"),
            "Exigences extraites du document",
            "",
        ));
        let requirements: Vec<&str> = text
            .split(['.', '\n'])
            .map(str::trim)
            .filter(|s| s.split_whitespace().count() >= 5 && REQUIREMENT.is_match(s))
            .take(20)
            .collect();
        if requirements.is_empty() {
            nodes.push(make_task(
                "E1-T1".into(),
                "E1",
                &goal,
                &short(text, 400),
                vec![],
            ));
        }
        for (i, requirement) in requirements.iter().enumerate() {
            nodes.push(make_task(
                format!("E1-T{}", i + 1),
                "E1",
                &short(requirement, 90),
                requirement,
                vec![],
            ));
        }
        link_foundations(&mut nodes, "E1");
    } else {
        // Cross-epic rule: user-facing work depends on the authentication / foundation epic.
        let foundation_epics: Vec<String> = nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Epic && FOUNDATION_EPIC.is_match(&n.title))
            .map(|n| n.id.clone())
            .collect();
        for node in nodes.iter_mut().filter(|n| n.kind == NodeKind::Task) {
            let parent = node.parent.clone().unwrap_or_default();
            if foundation_epics.contains(&parent) {
                continue;
            }
            let scope = format!("{} {}", node.title, node.acceptance_criteria.join(" "));
            if USER_SCOPED.is_match(&scope) {
                node.depends_on.extend(foundation_epics.iter().cloned());
            }
        }
    }
    empty_plan(3, goal, nodes, confidence, "heuristic")
}

// ---------------------------------------------------------------------------------------
// LLM planning (Product Owner agent)
// ---------------------------------------------------------------------------------------

const PLANNER_SYSTEM: &str = r#"You are the Product Owner of an autonomous software factory.
You turn a request (micro-task, epic or full specification) into a Hierarchical Task Network
that engineering agents will execute with strict test-driven development.

Rules:
- Decompose top-down: one goal → epics → (stories) → tasks. Leaves must be small enough to
  be implemented and verified by automated tests in one TDD cycle.
- Every leaf has concrete, testable acceptance criteria (observable behaviour, not intentions).
- `depends_on` lists the ids that must be finished first. Only real technical dependencies:
  no artificial chains, no cycles.
- Estimate each leaf in story points (1, 2, 3, 5, 8) and assess its risk: probability and
  impact on a 1-5 scale, the risk factors, and a mitigation.
- Keep the wording of titles and criteria in the language of the request.
- Do not invent requirements that the request does not imply; list assumptions in the
  description of the goal node instead.

Answer with a single JSON object and nothing else:
{
  "goal": "one-sentence objective",
  "confidence": 0.0-1.0,
  "nodes": [
    {"id": "G", "kind": "goal", "parent": null, "title": "...", "description": "..."},
    {"id": "E1", "kind": "epic", "parent": "G", "title": "...", "description": "..."},
    {"id": "E1-T1", "kind": "task", "parent": "E1", "title": "...", "description": "...",
     "component": "backend|frontend|infra|docs",
     "acceptance_criteria": ["..."], "depends_on": [], "estimate_points": 3,
     "risk": {"probability": 2, "impact": 3, "factors": ["..."], "mitigation": "..."}}
  ]
}"#;

/// Messages for the structured planning call.
pub fn planning_messages(text: &str, level: u8, feedback: Option<&str>) -> Vec<ChatMessage> {
    let granularity = match level {
        1 => "Level 1 — micro-task: a goal and usually a single task (split only if it is not atomic).",
        2 => "Level 2 — functional epic: a goal, one epic and its tasks.",
        _ => "Level 3 — full specification: a goal, several epics and their tasks.",
    };
    let mut user =
        format!("Input granularity: {granularity}\n\n<specification>\n{text}\n</specification>\n");
    if let Some(feedback) = feedback {
        user.push_str(&format!(
            "\nA human supervisor reviewed your previous plan and requested changes:\n<feedback>\n{feedback}\n</feedback>\nProduce a revised plan that addresses this feedback.\n"
        ));
    }
    vec![ChatMessage::system(PLANNER_SYSTEM), ChatMessage::user(user)]
}

fn str_field(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().trim().to_string()
}

fn str_list(v: &Value, key: &str) -> Vec<String> {
    v[key]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Parses and validates a model-produced plan. Tolerant on form, strict on substance.
pub fn parse_llm_plan(answer: &str, level: u8, sprint_capacity: u32) -> Result<Plan> {
    let json = extract_json(answer)
        .ok_or_else(|| Error::Llm("the planner did not return a JSON object".into()))?;
    let raw_nodes = json["nodes"]
        .as_array()
        .ok_or_else(|| Error::Llm("the plan has no `nodes` array".into()))?;

    let mut nodes = Vec::new();
    for raw in raw_nodes {
        let id = str_field(raw, "id");
        let title = str_field(raw, "title");
        if id.is_empty() || title.is_empty() {
            continue;
        }
        let kind = match str_field(raw, "kind").to_ascii_lowercase().as_str() {
            "goal" => NodeKind::Goal,
            "epic" => NodeKind::Epic,
            "story" => NodeKind::Story,
            _ => NodeKind::Task,
        };
        let description = str_field(raw, "description");
        let criteria = str_list(raw, "acceptance_criteria");
        let scope = format!("{title} {description} {}", criteria.join(" "));
        let risk = match (
            raw["risk"]["probability"].as_u64(),
            raw["risk"]["impact"].as_u64(),
        ) {
            (Some(p), Some(i)) => Risk::new(
                p.clamp(1, 5) as u8,
                i.clamp(1, 5) as u8,
                str_list(&raw["risk"], "factors"),
                str_field(&raw["risk"], "mitigation"),
            ),
            _ => assess_risk(&scope),
        };
        let component = match str_field(raw, "component").to_ascii_lowercase().as_str() {
            "frontend" => Component::Frontend,
            "infra" => Component::Infra,
            "docs" => Component::Docs,
            "backend" => Component::Backend,
            _ => detect_component(&title),
        };
        let estimate = raw["estimate_points"]
            .as_u64()
            .map(|p| p.clamp(1, 13) as u32)
            .unwrap_or_else(|| estimate_points(criteria.len(), &risk));
        nodes.push(TaskNode {
            id,
            title,
            description,
            kind,
            parent: raw["parent"]
                .as_str()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            acceptance_criteria: criteria,
            depends_on: str_list(raw, "depends_on"),
            component,
            estimate_points: estimate,
            risk,
        });
    }

    let goal = {
        let stated = str_field(&json, "goal");
        if !stated.is_empty() {
            stated
        } else {
            nodes
                .iter()
                .find(|n| n.kind == NodeKind::Goal)
                .map(|n| n.title.clone())
                .unwrap_or_default()
        }
    };
    let confidence = json["confidence"].as_f64().unwrap_or(0.7).clamp(0.0, 1.0);
    let mut plan = empty_plan(level, goal, nodes, confidence, "llm");
    plan.finalize(sprint_capacity);

    let leaves = plan.leaves();
    if leaves.is_empty() {
        return Err(Error::Llm("the plan contains no executable task".into()));
    }
    if let Some(vague) = leaves.iter().find(|n| n.acceptance_criteria.is_empty()) {
        return Err(Error::Llm(format!(
            "task '{}' has no acceptance criteria",
            vague.id
        )));
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRD: &str = "# Plateforme de réservation de salles\n\nContexte général du projet.\n\n\
        ## Authentification\nGestion des comptes.\n- Inscription par e-mail\n  - Le mot de passe est haché\n  - Un e-mail en double est refusé\n- Connexion avec jeton JWT\n\n\
        ## Réservations\n- Modèle de données des salles\n- Création d'une réservation par un utilisateur\n- Tableau de bord des réservations\n\n\
        ## Paiement\n- Paiement en ligne avec Stripe\n- Webhooks de confirmation\n";

    #[test]
    fn micro_task_becomes_a_single_tdd_task() {
        let plan = heuristic_plan("Créer un endpoint FastAPI sécurisé avec JWT", 1, 13);
        assert_eq!(plan.execution_order, vec!["T1"]);
        let task = plan.node("T1").unwrap();
        assert!(!task.acceptance_criteria.is_empty());
        assert!(task.risk.factors.iter().any(|f| f.contains("sécurité")));
    }

    #[test]
    fn epic_is_split_into_a_foundation_and_its_deliverables() {
        let plan = heuristic_plan(
            "Implémenter le module de facturation avec Stripe, webhooks et dashboard",
            2,
            13,
        );
        assert_eq!(plan.execution_order.len(), 4);
        assert_eq!(plan.waves[0], vec!["T1"]);
        assert_eq!(plan.waves[1].len(), 3);
        assert_eq!(plan.node("T4").unwrap().component, Component::Frontend);
        assert!(plan.node("T2").unwrap().risk.score > plan.node("T4").unwrap().risk.score);
    }

    #[test]
    fn prd_becomes_a_tree_with_dependencies_risks_and_sprints() {
        let plan = heuristic_plan(PRD, 3, 8);
        assert_eq!(plan.goal, "Plateforme de réservation de salles");
        let epics: Vec<_> = plan
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Epic)
            .map(|n| n.title.as_str())
            .collect();
        assert_eq!(epics, vec!["Authentification", "Réservations", "Paiement"]);
        assert_eq!(plan.leaves().len(), 7);
        // Nested bullets become acceptance criteria.
        assert_eq!(plan.node("E1-T1").unwrap().acceptance_criteria.len(), 2);
        // In-epic foundation: the booking tasks wait for the data model.
        assert!(plan
            .node("E2-T2")
            .unwrap()
            .depends_on
            .contains(&"E2-T1".to_string()));
        // Cross-epic foundation: user-facing work waits for authentication.
        assert!(plan
            .node("E2-T2")
            .unwrap()
            .depends_on
            .contains(&"E1-T1".to_string()));
        assert!(plan.sprints.len() >= 2);
        assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
        assert!(plan.node("E3-T1").unwrap().risk.level() != "low");
    }

    #[test]
    fn unstructured_prose_falls_back_to_requirement_sentences() {
        let text = "Le système doit permettre l'export des rapports au format CSV. \
                    Les administrateurs doivent pouvoir révoquer un accès à tout moment. Merci.";
        let plan = heuristic_plan(text, 3, 13);
        assert_eq!(plan.leaves().len(), 2);
        assert!(plan.confidence < 0.7);
    }

    #[test]
    fn parses_a_model_plan_and_recomputes_scheduling() {
        let answer = r#"Voici le plan :
```json
{"goal": "API de notes", "confidence": 0.82, "nodes": [
  {"id": "G", "kind": "goal", "parent": null, "title": "API de notes"},
  {"id": "E1", "kind": "epic", "parent": "G", "title": "CRUD"},
  {"id": "E1-T2", "kind": "task", "parent": "E1", "title": "Endpoint de création", "depends_on": ["E1-T1"],
   "acceptance_criteria": ["POST /notes renvoie 201"], "estimate_points": 3,
   "risk": {"probability": 2, "impact": 2, "factors": ["validation"], "mitigation": "tests"}},
  {"id": "E1-T1", "kind": "feature", "parent": "E1", "title": "Modèle Note", "acceptance_criteria": ["Une note a un titre"]}
]}
```"#;
        let plan = parse_llm_plan(answer, 2, 13).unwrap();
        assert_eq!(plan.planner, "llm");
        assert_eq!(plan.execution_order, vec!["E1-T1", "E1-T2"]);
        assert_eq!(plan.node("E1-T2").unwrap().risk.score, 4);
        assert!((plan.confidence - 0.82).abs() < 1e-9);
    }

    #[test]
    fn rejects_model_plans_without_testable_tasks() {
        assert!(parse_llm_plan("pas de json", 1, 13).is_err());
        let vague =
            r#"{"goal": "x", "nodes": [{"id": "T1", "kind": "task", "title": "Faire le truc"}]}"#;
        assert!(parse_llm_plan(vague, 1, 13)
            .unwrap_err()
            .to_string()
            .contains("acceptance criteria"));
    }
}
