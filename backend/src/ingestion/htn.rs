//! Hierarchical Task Network: the plan model and the deterministic graph algorithms
//! applied to it (dependency validation, execution waves, critical path, sprint packing).
//!
//! LLMs propose the decomposition; they are never trusted with graph algorithms. Whatever
//! planner produced the nodes, [`Plan::finalize`] repairs and schedules them.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Goal,
    Epic,
    Story,
    Task,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Component {
    Backend,
    Frontend,
    Infra,
    Docs,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Risk {
    /// 1 (rare) … 5 (almost certain).
    pub probability: u8,
    /// 1 (negligible) … 5 (critical).
    pub impact: u8,
    /// probability × impact (1…25).
    pub score: u8,
    pub factors: Vec<String>,
    pub mitigation: String,
}

impl Default for Risk {
    fn default() -> Self {
        Self {
            probability: 1,
            impact: 2,
            score: 2,
            factors: vec![],
            mitigation: String::new(),
        }
    }
}

impl Risk {
    pub fn new(probability: u8, impact: u8, factors: Vec<String>, mitigation: String) -> Self {
        let (p, i) = (probability.clamp(1, 5), impact.clamp(1, 5));
        Self {
            probability: p,
            impact: i,
            score: p * i,
            factors,
            mitigation,
        }
    }

    pub fn level(&self) -> &'static str {
        match self.score {
            0..=5 => "low",
            6..=11 => "medium",
            _ => "high",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskNode {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub kind: NodeKind,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default = "default_component")]
    pub component: Component,
    #[serde(default = "default_points")]
    pub estimate_points: u32,
    #[serde(default)]
    pub risk: Risk,
}

fn default_component() -> Component {
    Component::Backend
}

fn default_points() -> u32 {
    2
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sprint {
    pub id: u32,
    pub name: String,
    pub task_ids: Vec<String>,
    pub points: u32,
    pub max_risk_score: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    /// Input granularity: 1 micro-task, 2 epic, 3 full PRD.
    pub level: u8,
    pub goal: String,
    /// The whole tree: goal → epics → stories → tasks. Leaves are the executable tasks.
    pub nodes: Vec<TaskNode>,
    /// Leaves in a valid execution order (topological, stable).
    #[serde(default)]
    pub execution_order: Vec<String>,
    /// Leaves grouped by dependency depth: tasks of one wave are mutually independent.
    #[serde(default)]
    pub waves: Vec<Vec<String>>,
    /// Longest dependency chain, weighted by estimates.
    #[serde(default)]
    pub critical_path: Vec<String>,
    #[serde(default)]
    pub sprints: Vec<Sprint>,
    #[serde(default)]
    pub total_points: u32,
    /// Planner's confidence in the decomposition (0..1).
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    /// `heuristic` or `llm`.
    #[serde(default)]
    pub planner: String,
    /// Repairs applied while validating the graph.
    #[serde(default)]
    pub warnings: Vec<String>,
}

fn default_confidence() -> f64 {
    0.7
}

impl Plan {
    pub fn node(&self, id: &str) -> Option<&TaskNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Executable tasks: nodes without children (the goal node excluded).
    pub fn leaves(&self) -> Vec<&TaskNode> {
        let parents: BTreeSet<&str> = self
            .nodes
            .iter()
            .filter_map(|n| n.parent.as_deref())
            .collect();
        self.nodes
            .iter()
            .filter(|n| !parents.contains(n.id.as_str()) && n.kind != NodeKind::Goal)
            .collect()
    }

    fn descendants_leaves(&self, id: &str, leaf_ids: &BTreeSet<String>) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![id.to_string()];
        while let Some(current) = stack.pop() {
            for child in self
                .nodes
                .iter()
                .filter(|n| n.parent.as_deref() == Some(current.as_str()))
            {
                if leaf_ids.contains(&child.id) {
                    out.push(child.id.clone());
                } else {
                    stack.push(child.id.clone());
                }
            }
        }
        out
    }

    /// Validates and repairs the dependency graph, then computes the execution order,
    /// parallel waves, critical path and sprint plan. Idempotent.
    pub fn finalize(&mut self, sprint_capacity: u32) {
        self.warnings.clear();

        // 1. Unique ids and existing parents.
        let mut seen = BTreeSet::new();
        self.nodes
            .retain(|n| !n.id.trim().is_empty() && seen.insert(n.id.clone()));
        let ids: BTreeSet<String> = self.nodes.iter().map(|n| n.id.clone()).collect();
        for node in &mut self.nodes {
            if node
                .parent
                .as_ref()
                .is_some_and(|p| !ids.contains(p) || *p == node.id)
            {
                node.parent = None;
            }
        }

        // 2. Dependencies only between leaves: a dependency on a composite node expands to
        //    its leaves; a composite node's dependencies are pushed down to its leaves.
        let leaf_ids: BTreeSet<String> = self.leaves().iter().map(|n| n.id.clone()).collect();
        let mut inherited: HashMap<String, Vec<String>> = HashMap::new();
        for node in self
            .nodes
            .iter()
            .filter(|n| !leaf_ids.contains(&n.id) && !n.depends_on.is_empty())
        {
            for leaf in self.descendants_leaves(&node.id, &leaf_ids) {
                inherited
                    .entry(leaf)
                    .or_default()
                    .extend(node.depends_on.iter().cloned());
            }
        }
        let mut resolved: HashMap<String, Vec<String>> = HashMap::new();
        let mut warnings = Vec::new();
        for node in self.nodes.iter().filter(|n| leaf_ids.contains(&n.id)) {
            let mut deps: Vec<String> = Vec::new();
            let raw = node
                .depends_on
                .iter()
                .chain(inherited.get(&node.id).into_iter().flatten());
            for dep in raw {
                if leaf_ids.contains(dep) {
                    deps.push(dep.clone());
                } else if ids.contains(dep) {
                    deps.extend(self.descendants_leaves(dep, &leaf_ids));
                } else {
                    warnings.push(format!(
                        "{} : dépendance inconnue « {dep} » ignorée",
                        node.id
                    ));
                }
            }
            let mut unique = BTreeSet::new();
            deps.retain(|d| *d != node.id && unique.insert(d.clone()));
            resolved.insert(node.id.clone(), deps);
        }
        for node in &mut self.nodes {
            node.depends_on = resolved.remove(&node.id).unwrap_or_default();
        }

        // 3. Kahn's algorithm, wave by wave. Cycles are broken by dropping the offending edges.
        let order: Vec<String> = self
            .nodes
            .iter()
            .filter(|n| leaf_ids.contains(&n.id))
            .map(|n| n.id.clone())
            .collect();
        let mut deps: BTreeMap<String, BTreeSet<String>> = self
            .nodes
            .iter()
            .filter(|n| leaf_ids.contains(&n.id))
            .map(|n| (n.id.clone(), n.depends_on.iter().cloned().collect()))
            .collect();
        let mut done: BTreeSet<String> = BTreeSet::new();
        let mut waves: Vec<Vec<String>> = Vec::new();
        while done.len() < order.len() {
            let ready: Vec<String> = order
                .iter()
                .filter(|id| !done.contains(*id) && deps[*id].iter().all(|d| done.contains(d)))
                .cloned()
                .collect();
            if ready.is_empty() {
                // Cycle: release the first blocked task by dropping its unresolved dependencies.
                let victim = order
                    .iter()
                    .find(|id| !done.contains(*id))
                    .expect("pending task")
                    .clone();
                let dropped: Vec<String> = deps[&victim]
                    .iter()
                    .filter(|d| !done.contains(*d))
                    .cloned()
                    .collect();
                warnings.push(format!(
                    "cycle de dépendances rompu : {victim} ne dépend plus de {}",
                    dropped.join(", ")
                ));
                deps.get_mut(&victim)
                    .expect("known task")
                    .retain(|d| done.contains(d));
                if let Some(node) = self.nodes.iter_mut().find(|n| n.id == victim) {
                    node.depends_on.retain(|d| !dropped.contains(d));
                }
                continue;
            }
            done.extend(ready.iter().cloned());
            waves.push(ready);
        }
        self.execution_order = waves.iter().flatten().cloned().collect();
        self.waves = waves;

        // 4. Critical path: longest chain by estimate, computed in topological order.
        let points: HashMap<String, u32> = self
            .nodes
            .iter()
            .map(|n| (n.id.clone(), n.estimate_points.max(1)))
            .collect();
        let mut best: HashMap<String, (u32, Option<String>)> = HashMap::new();
        for id in &self.execution_order {
            let (cost, pred) = deps[id]
                .iter()
                .filter_map(|d| best.get(d).map(|(c, _)| (*c, Some(d.clone()))))
                .max_by_key(|(c, _)| *c)
                .unwrap_or((0, None));
            best.insert(id.clone(), (cost + points[id], pred));
        }
        let mut path = Vec::new();
        let mut cursor = self
            .execution_order
            .iter()
            .max_by_key(|id| {
                (
                    best[*id].0,
                    std::cmp::Reverse(self.execution_order.iter().position(|x| x == *id)),
                )
            })
            .cloned();
        while let Some(id) = cursor {
            cursor = best[&id].1.clone();
            path.push(id);
        }
        path.reverse();
        self.critical_path = path;

        // 5. Sprint packing: a task goes into the earliest sprint that is not before any of
        //    its dependencies' sprints and still has capacity.
        let capacity = sprint_capacity.max(1);
        let mut sprints: Vec<Sprint> = Vec::new();
        let mut placed: HashMap<String, usize> = HashMap::new();
        for id in &self.execution_order {
            let pts = points[id];
            let earliest = deps[id]
                .iter()
                .filter_map(|d| placed.get(d))
                .max()
                .copied()
                .unwrap_or(0);
            let mut idx = earliest;
            loop {
                if idx == sprints.len() {
                    sprints.push(Sprint {
                        id: idx as u32 + 1,
                        name: format!("Sprint {}", idx + 1),
                        task_ids: vec![],
                        points: 0,
                        max_risk_score: 0,
                    });
                }
                // An oversized task still gets a sprint of its own.
                if sprints[idx].points + pts <= capacity || sprints[idx].task_ids.is_empty() {
                    break;
                }
                idx += 1;
            }
            let risk = self.node(id).map(|n| n.risk.score).unwrap_or(0);
            sprints[idx].task_ids.push(id.clone());
            sprints[idx].points += pts;
            sprints[idx].max_risk_score = sprints[idx].max_risk_score.max(risk);
            placed.insert(id.clone(), idx);
        }
        self.sprints = sprints;
        self.total_points = self.execution_order.iter().map(|id| points[id]).sum();
        self.warnings = warnings;
    }

    /// Indented outline of the tree, for prompts and the CLI.
    pub fn outline(&self) -> String {
        fn walk(plan: &Plan, parent: Option<&str>, depth: usize, out: &mut String) {
            for node in plan.nodes.iter().filter(|n| n.parent.as_deref() == parent) {
                let deps = if node.depends_on.is_empty() {
                    String::new()
                } else {
                    format!("  ← {}", node.depends_on.join(", "))
                };
                out.push_str(&format!(
                    "{}- [{}] {} ({} pts, risque {}){}\n",
                    "  ".repeat(depth),
                    node.id,
                    node.title,
                    node.estimate_points,
                    node.risk.score,
                    deps
                ));
                walk(plan, Some(&node.id), depth + 1, out);
            }
        }
        let mut out = String::new();
        walk(self, None, 0, &mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, parent: &str, deps: &[&str], points: u32) -> TaskNode {
        TaskNode {
            id: id.into(),
            title: id.into(),
            description: String::new(),
            kind: NodeKind::Task,
            parent: Some(parent.into()),
            acceptance_criteria: vec![],
            depends_on: deps.iter().map(|d| d.to_string()).collect(),
            component: Component::Backend,
            estimate_points: points,
            risk: Risk::default(),
        }
    }

    fn plan(nodes: Vec<TaskNode>) -> Plan {
        let mut all = vec![TaskNode {
            kind: NodeKind::Goal,
            parent: None,
            ..task("G", "", &[], 0)
        }];
        all.push(TaskNode {
            kind: NodeKind::Epic,
            parent: Some("G".into()),
            ..task("E1", "G", &[], 0)
        });
        all.extend(nodes);
        Plan {
            level: 3,
            goal: "goal".into(),
            nodes: all,
            execution_order: vec![],
            waves: vec![],
            critical_path: vec![],
            sprints: vec![],
            total_points: 0,
            confidence: 0.8,
            planner: "test".into(),
            warnings: vec![],
        }
    }

    #[test]
    fn orders_tasks_and_builds_waves() {
        let mut p = plan(vec![
            task("C", "E1", &["A", "B"], 3),
            task("A", "E1", &[], 2),
            task("B", "E1", &["A"], 5),
        ]);
        p.finalize(13);
        assert_eq!(p.execution_order, vec!["A", "B", "C"]);
        assert_eq!(p.waves, vec![vec!["A"], vec!["B"], vec!["C"]]);
        assert_eq!(p.critical_path, vec!["A", "B", "C"]);
        assert_eq!(p.total_points, 10);
        assert!(p.warnings.is_empty());
    }

    #[test]
    fn breaks_cycles_and_drops_unknown_dependencies() {
        let mut p = plan(vec![
            task("A", "E1", &["B", "ghost"], 1),
            task("B", "E1", &["A"], 1),
        ]);
        p.finalize(13);
        assert_eq!(p.execution_order.len(), 2);
        assert_eq!(p.warnings.len(), 2);
        let pos = |id: &str| p.execution_order.iter().position(|x| x == id).unwrap();
        for node in p.leaves() {
            for dep in &node.depends_on {
                assert!(pos(dep) < pos(&node.id));
            }
        }
    }

    #[test]
    fn sprints_respect_capacity_and_dependencies() {
        let mut p = plan(vec![
            task("A", "E1", &[], 8),
            task("B", "E1", &[], 8),
            task("C", "E1", &["B"], 3),
            task("D", "E1", &[], 21),
        ]);
        p.finalize(13);
        let sprint_of = |id: &str| {
            p.sprints
                .iter()
                .find(|s| s.task_ids.iter().any(|t| t == id))
                .unwrap()
                .id
        };
        assert_eq!(sprint_of("A"), 1);
        assert_eq!(sprint_of("B"), 2);
        assert!(sprint_of("C") >= sprint_of("B"));
        assert!(p
            .sprints
            .iter()
            .all(|s| s.points <= 13 || s.task_ids.len() == 1));
        assert_eq!(p.sprints.iter().map(|s| s.task_ids.len()).sum::<usize>(), 4);
    }

    #[test]
    fn dependencies_on_an_epic_expand_to_its_tasks() {
        let mut nodes = vec![task("A", "E1", &[], 1), task("B", "E1", &[], 1)];
        nodes.push(TaskNode {
            kind: NodeKind::Epic,
            parent: Some("G".into()),
            ..task("E2", "G", &[], 0)
        });
        nodes.push(task("Z", "E2", &["E1"], 1));
        let mut p = plan(nodes);
        p.finalize(13);
        assert_eq!(p.node("Z").unwrap().depends_on, vec!["A", "B"]);
        assert_eq!(p.waves.len(), 2);
    }
}
