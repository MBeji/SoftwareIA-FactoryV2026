// French labels and formatting helpers shared by every view.

import type {
  ExceptionKind,
  GateDecision,
  Phase,
  RunStatus,
  Severity,
  Stage,
  SupervisionMode,
  TaskStatus,
  Verdict,
} from "./types";

export const STATUS_LABEL: Record<RunStatus, string> = {
  pending: "En attente",
  running: "En cours",
  waiting_approval: "Validation requise",
  completed: "Terminé",
  failed: "Échec",
  cancelled: "Annulé",
  interrupted: "Interrompu",
};

export const PHASES: { id: Phase; label: string; hint: string }[] = [
  { id: "ingestion", label: "Ingestion", hint: "Lecture de la demande et classification en 3 niveaux" },
  { id: "planning", label: "Plan", hint: "Décomposition HTN, dépendances, risques, sprints" },
  { id: "architecture", label: "Architecture", hint: "Découpage en modules, interfaces, stratégie de test" },
  { id: "implementation", label: "TDD", hint: "Un cycle RED → GREEN → REFACTOR par tâche" },
  { id: "audit", label: "Audit", hint: "Sécurité et revue de code en parallèle" },
  { id: "delivery", label: "Livraison", hint: "Vérification finale et merge request" },
];

export const PHASE_LABEL: Record<Phase, string> = {
  ingestion: "Ingestion",
  planning: "Planification",
  architecture: "Architecture",
  implementation: "Implémentation (TDD)",
  audit: "Audit",
  delivery: "Livraison",
  done: "Terminé",
};

export const MODE_LABEL: Record<SupervisionMode, string> = {
  hitl: "HITL",
  hotl: "HOTL",
  autonomous: "Autonome",
};

export const MODE_LONG: Record<SupervisionMode, { name: string; tagline: string; detail: string }> = {
  hitl: {
    name: "Human-In-The-Loop",
    tagline: "Supervisé",
    detail: "Pause obligatoire à chaque étape clé : plan, architecture, plan de test, merge request.",
  },
  hotl: {
    name: "Human-On-The-Loop",
    tagline: "Supervision par exception",
    detail: "Les agents avancent seuls et ne s'arrêtent que sur exception : confiance basse, sécurité, divergence TDD.",
  },
  autonomous: {
    name: "Autonome",
    tagline: "Zéro contact",
    detail: "Aucune attente humaine jusqu'à la merge request. Une exception bloquante fait échouer le run.",
  },
};

export const STAGE_LABEL: Record<Stage, string> = {
  plan: "Plan",
  architecture: "Architecture",
  test_plan: "Plan de test",
  implementation: "Implémentation",
  security: "Sécurité",
  merge: "Merge request",
  budget: "Budget",
};

export const STAGE_HINT: Record<Stage, string> = {
  plan: "Validation de la décomposition en tâches",
  architecture: "Validation du document d'architecture",
  test_plan: "Validation des tests écrits avant le code (RED)",
  implementation: "Évalué sur exception : divergence de la boucle TDD",
  security: "Évalué sur exception : findings bloquants non résolus",
  merge: "Validation de la merge request finale",
  budget: "Évalué sur exception : plafond de dépense LLM atteint",
};

export const TASK_STATUS_LABEL: Record<TaskStatus, string> = {
  pending: "À faire",
  red: "RED",
  green: "GREEN",
  refactor: "REFACTOR",
  done: "Livrée",
  failed: "Échec",
};

export const VERDICT_LABEL: Record<Verdict, string> = {
  auto_approved: "Poursuite automatique",
  human_required: "Validation humaine requise",
  blocked: "Bloqué (échec du run)",
};

export const EXCEPTION_LABEL: Record<ExceptionKind, string> = {
  low_confidence: "Confiance basse",
  security_violation: "Violation de sécurité",
  tdd_divergence: "Divergence TDD",
  budget_exceeded: "Budget dépassé",
  policy_violation: "Violation de politique",
  vacuous_tests: "Tests sans valeur",
};

export const SEVERITY_LABEL: Record<Severity, string> = {
  info: "Info",
  low: "Faible",
  medium: "Moyenne",
  high: "Élevée",
  critical: "Critique",
};

export const DECISION_LABEL: Record<GateDecision, string> = {
  approve: "Approuvé",
  request_changes: "Modifications demandées",
  reject: "Rejeté",
};

export const KIND_LABEL: Record<string, string> = {
  classification: "Classification",
  planning: "Planification",
  architecture: "Architecture",
  complex_code: "Code complexe",
  code: "Code",
  test_generation: "Génération de tests",
  critique: "Critique / diagnostic",
  security_review: "Revue de sécurité",
  lint_fix: "Correction de lint",
  summarization: "Résumé",
};

export const AGENT_PHASE_LABEL: Record<string, string> = {
  plan: "planifie",
  architecture: "conçoit l'architecture",
  red: "écrit les tests (RED)",
  green: "implémente (GREEN)",
  reflect: "diagnostique l'échec",
  refactor: "nettoie le code (REFACTOR)",
  security_fix: "corrige un finding de sécurité",
  security_review: "audite la sécurité",
  review: "relit le code",
  revision: "applique une révision",
};

export const COMPONENT_LABEL: Record<string, string> = {
  backend: "Backend",
  frontend: "Frontend",
  infra: "Infra",
  docs: "Docs",
};

export function formatCost(usd: number): string {
  if (!Number.isFinite(usd) || usd === 0) return "0 $";
  return usd < 1 ? `${usd.toFixed(4)} $` : `${usd.toFixed(2)} $`;
}

export function formatTokens(count: number): string {
  if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1)} M`;
  if (count >= 1_000) return `${(count / 1_000).toFixed(1)} k`;
  return String(count);
}

export function formatPercent(ratio: number): string {
  return `${Math.round(ratio * 100)} %`;
}

export function formatTime(iso: string): string {
  const date = new Date(iso);
  return Number.isNaN(date.getTime())
    ? ""
    : date.toLocaleTimeString("fr-FR", { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

export function formatDateTime(iso: string): string {
  const date = new Date(iso);
  return Number.isNaN(date.getTime())
    ? ""
    : date.toLocaleString("fr-FR", { day: "2-digit", month: "short", hour: "2-digit", minute: "2-digit" });
}

export function riskLevel(score: number): "low" | "medium" | "high" {
  if (score <= 5) return "low";
  if (score <= 11) return "medium";
  return "high";
}

export function roleFromSource(source: string): string | null {
  return source.startsWith("agent:") ? source.slice("agent:".length) : null;
}

export function truncate(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max - 1).trimEnd()}…` : text;
}
