"use client";

import {
  ArrowRight,
  Ban,
  Brain,
  CircleCheck,
  CirclePause,
  CircleX,
  Eye,
  GitFork,
  GraduationCap,
  Link2,
  Play,
  Repeat,
  Route,
  Split,
  Zap,
} from "lucide-react";
import { useState, type CSSProperties, type ReactNode } from "react";

import { ConfidenceBar, MODE_TONE } from "@/components/status";
import { Badge } from "@/components/ui/badge";
import { Panel, PanelHeader } from "@/components/ui/panel";
import { Segmented } from "@/components/ui/segmented";
import {
  AGENT_PHASE_LABEL,
  EXCEPTION_LABEL,
  KIND_LABEL,
  MODE_LABEL,
  STAGE_LABEL,
  VERDICT_LABEL,
  formatCost,
  truncate,
} from "@/lib/format";
import { isActive, type OtaStep, type ViewState } from "@/lib/reducer";
import type { Evaluation, ExceptionKind, Meta, RunStatus, SupervisionMode, Verdict } from "@/lib/types";
import { cn } from "@/lib/utils";

// ------------------------------------------------------------------ narration

const PHASE_EXPLANATION: Record<string, string> = {
  plan: "Il décompose la demande en tâches vérifiables, avec dépendances, estimations et risques. L'ordonnancement est ensuite recalculé de façon déterministe.",
  architecture: "Il explore le workspace puis consigne l'architecture minimale qui couvre le plan, avec ses compromis.",
  red: "Les tests sont écrits avant le code. Ils doivent échouer tant que la fonctionnalité n'existe pas ; ils seront ensuite verrouillés.",
  green: "Le développeur écrit le code minimal qui fait passer les tests verrouillés. Le verdict final appartient à l'orchestrateur.",
  reflect: "Un modèle critique, d'un autre palier que le développeur, analyse la trace d'échec pour identifier la cause racine.",
  refactor: "Nettoyage sans changement de comportement : il n'est conservé que si les tests restent verts.",
  security_review: "Il relit le changement comme un attaquant, en complément du scanner statique déterministe.",
  security_fix: "Il corrige les findings bloquants ; les tests doivent rester verts, sinon le correctif est annulé.",
  review: "Dernier contrôle qualité : critères d'acceptation, pertinence des tests, lisibilité.",
  revision: "Il applique les modifications demandées ; elles ne sont gardées que si les tests restent verts.",
};

function narrate(view: ViewState, status: RunStatus, roleName: (id: string) => string): { headline: string; detail: string } {
  const { run, agents } = view;
  const pending = run.gates.find((gate) => !gate.resolution);
  switch (status) {
    case "waiting_approval":
      return {
        headline: `Le run attend votre décision : « ${pending?.request.title ?? "validation"} ».`,
        detail: pending?.request.reason ?? "Ouvrez le portail de validation pour approuver, demander des modifications ou rejeter.",
      };
    case "completed":
      return {
        headline: "Run terminé : la merge request est prête.",
        detail: "Chaque tâche est passée par RED → GREEN → REFACTOR, puis l'audit et la vérification finale. Voir l'onglet Livrable.",
      };
    case "failed":
      return { headline: "Run en échec.", detail: run.error ?? "L'usine s'est arrêtée sur une exception bloquante." };
    case "cancelled":
      return { headline: "Run arrêté par décision humaine.", detail: run.error ?? "Le workspace et le journal restent consultables." };
    case "interrupted":
      return {
        headline: "Run interrompu.",
        detail: "Le serveur s'est arrêté pendant l'exécution. Le journal d'événements permet de rejouer tout ce qui s'est passé.",
      };
    default:
      break;
  }
  const active = run.active_agents;
  if (active.length > 1) {
    return {
      headline: `${active.map(roleName).join(" et ")} travaillent en parallèle.`,
      detail: "Ces rôles sont en lecture seule et indépendants : ils examinent le même changement simultanément (pattern de parallélisation).",
    };
  }
  if (active.length === 1) {
    const activity = agents[active[0]];
    const phase = activity?.phase ?? "";
    const retry = phase === "green" && (activity?.attempt ?? 1) > 1;
    return {
      headline: `${roleName(active[0])} ${AGENT_PHASE_LABEL[phase] ?? "travaille"}${activity?.taskId ? ` pour ${activity.taskId}` : ""}.`,
      detail: retry
        ? `Tentative ${activity?.attempt} : le développeur repart du diagnostic de la boucle d'auto-correction.`
        : (PHASE_EXPLANATION[phase] ?? "L'agent avance par itérations Observer → Réfléchir → Agir."),
    };
  }
  if (run.phase === "ingestion") {
    return {
      headline: "L'orchestrateur prépare la sandbox et classe la demande.",
      detail: "Le niveau de granularité (micro-tâche, epic, cahier des charges) détermine la décomposition qui suit.",
    };
  }
  return {
    headline: "L'orchestrateur vérifie le travail avant de passer la main.",
    detail: "Tests, lint, scan de sécurité et commits sont exécutés par l'orchestrateur lui-même, dans la sandbox : il ne croit aucun agent sur parole.",
  };
}

// ------------------------------------------------------------------- OTA loop

const OTA: { id: OtaStep; label: string; icon: ReactNode; on: string; caption: string }[] = [
  { id: "think", label: "Réfléchir", icon: <Brain />, on: "border-violet bg-violet/15 text-violet", caption: "L'agent raisonne et annonce son intention" },
  { id: "act", label: "Agir", icon: <Zap />, on: "border-accent bg-accent/15 text-accent", caption: "Il appelle un outil" },
  { id: "observe", label: "Observer", icon: <Eye />, on: "border-ok bg-ok/15 text-ok", caption: "Le résultat revient et nourrit la suite" },
];

function OtaLoop({ view, running, roleName }: { view: ViewState; running: boolean; roleName: (id: string) => string }) {
  const ota = view.ota;
  const current = OTA.find((step) => step.id === ota?.step);
  return (
    <div>
      <div className="flex items-center gap-1">
        {OTA.map((step, index) => {
          const lit = running && ota?.step === step.id;
          return (
            <div key={step.id} className="flex min-w-0 flex-1 items-center gap-1">
              <div
                className={cn(
                  "flex min-w-0 flex-1 flex-col items-center gap-1 rounded-md border px-1 py-2 text-center transition-colors [&_svg]:size-4",
                  lit ? cn(step.on, "animate-token") : "border-line text-faint",
                )}
                style={lit ? ({ "--token-color": "currentColor" } as CSSProperties) : undefined}
                aria-current={lit ? "step" : undefined}
              >
                {step.icon}
                <span className="text-[11px] font-semibold">{step.label}</span>
              </div>
              {index < OTA.length - 1 ? <ArrowRight className="size-3.5 shrink-0 text-faint" /> : null}
            </div>
          );
        })}
      </div>
      <p className="mt-1 flex items-center justify-center gap-1 text-[10px] text-faint">
        <Repeat className="size-3" /> la boucle se répète jusqu&apos;à ce que l&apos;agent conclue (outil <span className="font-mono">finish</span>)
      </p>
      {ota && current ? (
        <div className="mt-2 rounded-md border border-line bg-canvas p-2">
          <p className="text-[11px] text-faint">
            {running ? "Étape en cours" : "Dernière étape"} — <span className="text-dim">{current.caption}</span>
            {ota.role ? <span className="text-dim"> · {roleName(ota.role)}</span> : null}
          </p>
          <p className={cn("mt-1 line-clamp-4 text-xs leading-relaxed break-words text-ink", ota.step !== "think" && "font-mono text-[11px]")}>
            {ota.step === "think" ? `« ${truncate(ota.text, 320)} »` : truncate(ota.text, 320) || "(aucune sortie)"}
          </p>
        </div>
      ) : (
        <p className="mt-2 text-xs text-faint">La boucle s&apos;animera dès qu&apos;un agent prendra la parole.</p>
      )}
    </div>
  );
}

// ------------------------------------------------------------------ TDD cycle

const TDD = [
  { id: "red", label: "RED", color: "var(--color-bad)", text: "text-bad", caption: "QA écrit les tests ; ils doivent échouer" },
  { id: "green", label: "GREEN", color: "var(--color-ok)", text: "text-ok", caption: "Le dev écrit le code minimal" },
  { id: "refactor", label: "REFACTOR", color: "var(--color-violet)", text: "text-violet", caption: "Lint et nettoyage, tests toujours verts" },
] as const;

function arcPath(startDeg: number, endDeg: number, radius = 44, center = 56): string {
  const point = (deg: number) => {
    const rad = (deg * Math.PI) / 180;
    return `${(center + radius * Math.sin(rad)).toFixed(2)} ${(center - radius * Math.cos(rad)).toFixed(2)}`;
  };
  return `M ${point(startDeg)} A ${radius} ${radius} 0 0 1 ${point(endDeg)}`;
}

function TddCycle({ view, running }: { view: ViewState; running: boolean }) {
  const tdd = view.run.tdd;
  const taskId = tdd?.task_id ?? null;
  const phase = tdd?.phase ?? null;
  const task = view.run.plan?.nodes.find((node) => node.id === taskId);
  const verdicts = view.verdicts.filter((verdict) => verdict.taskId === taskId).slice(-5);
  const reflection = [...view.reflections].reverse().find((entry) => entry.taskId === taskId);

  return (
    <div>
      <div className="flex items-center gap-3">
        <svg viewBox="0 0 112 112" className="size-28 shrink-0" role="img" aria-label={`Cycle TDD : phase ${phase ?? "non démarrée"}`}>
          {TDD.map((step, index) => {
            const lit = phase === step.id && running;
            const past = phase === "done" || TDD.findIndex((candidate) => candidate.id === phase) > index;
            return (
              <path
                key={step.id}
                d={arcPath(index * 120 - 54, index * 120 + 54)}
                fill="none"
                stroke={step.color}
                strokeWidth={lit ? 11 : 7}
                strokeLinecap="round"
                opacity={lit ? 1 : past ? 0.6 : 0.2}
              >
                {lit ? <animate attributeName="opacity" values="1;0.55;1" dur="1.4s" repeatCount="indefinite" /> : null}
              </path>
            );
          })}
          <text x="56" y="53" textAnchor="middle" className="fill-ink font-mono text-[13px] font-semibold">
            {taskId ?? "TDD"}
          </text>
          <text x="56" y="68" textAnchor="middle" className="fill-dim text-[8.5px]">
            {phase === "done" ? "cycle terminé" : phase === "green" && tdd ? `tentative ${tdd.attempt}/${tdd.max_attempts}` : phase ? phase.toUpperCase() : "en attente"}
          </text>
        </svg>
        <ol className="min-w-0 flex-1 space-y-1">
          {TDD.map((step) => {
            const lit = phase === step.id && running;
            return (
              <li key={step.id} className={cn("rounded border px-2 py-1", lit ? "border-line-strong bg-raised" : "border-transparent")}>
                <span className={cn("text-[11px] font-bold", lit || phase === "done" ? step.text : "text-faint")}>{step.label}</span>
                <span className={cn("ml-1.5 text-[11px]", lit ? "text-ink" : "text-faint")}>{step.caption}</span>
              </li>
            );
          })}
        </ol>
      </div>
      {task ? <p className="mt-1.5 truncate text-[11px] text-dim" title={task.title}>Tâche : {task.title}</p> : null}

      {verdicts.length > 0 ? (
        <ul className="mt-2 space-y-1" aria-label="Verdicts de test rendus par l'orchestrateur">
          {verdicts.map((verdict) => (
            <li key={verdict.seq} className="flex items-start gap-1.5 rounded border border-line bg-canvas px-2 py-1 text-[11px]">
              {verdict.asExpected ? (
                <CircleCheck className="mt-0.5 size-3.5 shrink-0 text-ok" />
              ) : (
                <CircleX className="mt-0.5 size-3.5 shrink-0 text-bad" />
              )}
              <span className="min-w-0">
                <span className="font-mono text-dim">
                  {verdict.phase}
                  {verdict.phase === "green" ? ` #${verdict.attempt}` : ""}
                </span>{" "}
                <span className="text-faint">attendu : {verdict.expected === "fail" ? "échec" : "succès"} →</span>{" "}
                <span className={verdict.report.ok ? "text-ok" : "text-bad"}>{verdict.summary}</span>
              </span>
            </li>
          ))}
        </ul>
      ) : taskId ? (
        <p className="mt-2 text-xs text-faint">
          En attente du premier verdict : c&apos;est l&apos;orchestrateur qui exécute les tests dans la sandbox, pas l&apos;agent.
        </p>
      ) : (
        <p className="mt-2 text-xs text-faint">
          Le cycle démarre à la phase d&apos;implémentation : une boucle RED → GREEN → REFACTOR par tâche, dans l&apos;ordre des dépendances.
        </p>
      )}

      {reflection ? (
        <div className="mt-2 rounded-md border border-warn/30 bg-warn/8 p-2">
          <p className="flex items-center gap-1.5 text-[11px] font-semibold text-warn">
            <Repeat className="size-3.5" /> Boucle d&apos;auto-correction — diagnostic après la tentative {reflection.attempt}
          </p>
          <p className="mt-1 text-xs leading-relaxed text-ink">{reflection.rootCause}</p>
          {reflection.fixPlan ? <p className="mt-1 text-xs leading-relaxed text-dim">Correctif : {reflection.fixPlan}</p> : null}
        </div>
      ) : null}
    </div>
  );
}

// -------------------------------------------------------------- decision tree

type Answer = "key" | "exception" | "blocking";
type TreeNode =
  | { kind: "question"; ask: Answer; label: string; hint: string; yes: TreeNode; no: TreeNode }
  | { kind: "leaf"; verdict: Verdict; note?: string };

const leaf = (verdict: Verdict, note?: string): TreeNode => ({ kind: "leaf", verdict, note });

const TREES: Record<SupervisionMode, TreeNode> = {
  hitl: {
    kind: "question",
    ask: "key",
    label: "Étape clé ?",
    hint: "plan, architecture, plan de test, merge request",
    yes: leaf("human_required"),
    no: {
      kind: "question",
      ask: "exception",
      label: "Exception levée ?",
      hint: "confiance < seuil, sécurité, divergence TDD, budget, politique",
      yes: leaf("human_required"),
      no: leaf("auto_approved"),
    },
  },
  hotl: {
    kind: "question",
    ask: "exception",
    label: "Exception levée ?",
    hint: "confiance < seuil, sécurité, divergence TDD, budget, politique",
    yes: leaf("human_required"),
    no: leaf("auto_approved"),
  },
  autonomous: {
    kind: "question",
    ask: "blocking",
    label: "Exception bloquante ?",
    hint: "sécurité, divergence TDD, budget",
    yes: leaf("blocked"),
    no: leaf("auto_approved", "les exceptions non bloquantes sont consignées"),
  },
};

const BLOCKING: ExceptionKind[] = ["security_violation", "tdd_divergence", "budget_exceeded"];

const LEAF_STYLE: Record<Verdict, { icon: ReactNode; on: string }> = {
  auto_approved: { icon: <Play />, on: "border-ok bg-ok/15 text-ok" },
  human_required: { icon: <CirclePause />, on: "border-warn bg-warn/15 text-warn" },
  blocked: { icon: <Ban />, on: "border-bad bg-bad/15 text-bad" },
};

function TreeView({ node, answers, onPath }: { node: TreeNode; answers: Record<Answer, boolean> | null; onPath: boolean }) {
  if (node.kind === "leaf") {
    const style = LEAF_STYLE[node.verdict];
    return (
      <div
        className={cn(
          "inline-flex items-center gap-1.5 rounded-md border px-2 py-1 text-[11px] font-medium [&_svg]:size-3.5",
          onPath ? style.on : "border-line text-faint",
        )}
      >
        {style.icon}
        {VERDICT_LABEL[node.verdict]}
        {node.note ? <span className="font-normal opacity-80">({node.note})</span> : null}
      </div>
    );
  }
  const answer = onPath && answers ? answers[node.ask] : null;
  return (
    <div>
      <div className={cn("inline-block rounded-md border px-2 py-1", onPath ? "border-accent bg-accent/10" : "border-line")}>
        <p className={cn("text-[11px] font-semibold", onPath ? "text-accent" : "text-dim")}>{node.label}</p>
        <p className="text-[10px] text-faint">{node.hint}</p>
      </div>
      <div className="mt-1 ml-3 space-y-1 border-l border-line-strong pl-3">
        {(["yes", "no"] as const).map((branch) => {
          const taken = answer !== null && answer === (branch === "yes");
          return (
            <div key={branch} className="flex items-start gap-2">
              <span className={cn("mt-1.5 w-10 shrink-0 font-mono text-[10px] whitespace-nowrap", taken ? "font-bold text-accent" : "text-faint")}>
                {branch === "yes" ? "oui →" : "non →"}
              </span>
              <TreeView node={node[branch]} answers={answers} onPath={taken} />
            </div>
          );
        })}
      </div>
    </div>
  );
}

function DecisionTree({ view, meta }: { view: ViewState; meta: Meta }) {
  const [pinned, setPinned] = useState<SupervisionMode | null>(null);
  const evaluations = view.run.evaluations;
  const latest: Evaluation | null = evaluations.length > 0 ? evaluations[evaluations.length - 1] : null;
  const shown = pinned ?? latest?.mode ?? view.run.supervision.default_mode;
  const answers =
    latest && latest.mode === shown
      ? {
          key: Boolean(meta.stages.find((stage) => stage.id === latest.stage)?.key_checkpoint),
          exception: latest.exceptions.length > 0,
          blocking: latest.exceptions.some((exception) => BLOCKING.includes(exception.kind)),
        }
      : null;

  return (
    <div>
      <div className="mb-2 flex items-center justify-between gap-2">
        <span className="text-[11px] text-faint">Arbre du mode</span>
        <Segmented<SupervisionMode>
          label="Arbre de décision à afficher"
          value={shown}
          onChange={(mode) => setPinned(mode === latest?.mode ? null : mode)}
          options={(["hitl", "hotl", "autonomous"] as SupervisionMode[]).map((mode) => ({ value: mode, label: MODE_LABEL[mode] }))}
        />
      </div>
      <TreeView node={TREES[shown]} answers={answers} onPath={answers !== null} />

      {latest ? (
        <div className="mt-2.5 rounded-md border border-line bg-canvas p-2">
          <p className="flex flex-wrap items-center gap-1.5 text-[11px] text-faint">
            Dernier point de contrôle
            <Badge tone={MODE_TONE[latest.mode]}>{MODE_LABEL[latest.mode]}</Badge>
            <Badge>{STAGE_LABEL[latest.stage]}</Badge>
          </p>
          <p className="mt-1 text-xs font-medium text-ink">{latest.title}</p>
          <div className="mt-1.5 flex items-center gap-2 text-[11px] text-faint">
            Confiance <ConfidenceBar value={latest.confidence} threshold={latest.threshold} className="flex-1" />
          </div>
          {latest.exceptions.length > 0 ? (
            <div className="mt-1.5 flex flex-wrap gap-1">
              {latest.exceptions.map((exception, index) => (
                <Badge key={index} tone="warn" title={exception.detail}>
                  {EXCEPTION_LABEL[exception.kind] ?? exception.kind}
                </Badge>
              ))}
            </div>
          ) : null}
          <p
            className={cn(
              "mt-1.5 text-xs leading-relaxed",
              latest.verdict === "auto_approved" ? "text-ok" : latest.verdict === "blocked" ? "text-bad" : "text-warn",
            )}
          >
            → {VERDICT_LABEL[latest.verdict]} : {latest.reason}
          </p>
          {latest.mode !== shown ? (
            <p className="mt-1 text-[10px] text-faint">Ce point de contrôle a été évalué en {MODE_LABEL[latest.mode]} : sélectionnez ce mode pour voir son chemin.</p>
          ) : null}
        </div>
      ) : (
        <p className="mt-2 text-xs text-faint">Aucun point de contrôle évalué pour l&apos;instant. Le premier est la validation du plan.</p>
      )}

      {evaluations.length > 1 ? (
        <details className="mt-1.5">
          <summary className="cursor-pointer text-[11px] text-faint hover:text-dim">Historique ({evaluations.length} points de contrôle)</summary>
          <ul className="mt-1 space-y-0.5">
            {[...evaluations].reverse().slice(0, 12).map((evaluation) => (
              <li key={evaluation.seq} className="flex items-center gap-1.5 text-[11px]">
                <span className={cn("size-1.5 shrink-0 rounded-full", evaluation.verdict === "auto_approved" ? "bg-ok" : evaluation.verdict === "blocked" ? "bg-bad" : "bg-warn")} />
                <span className="truncate text-dim">{evaluation.title}</span>
                <span className="ml-auto shrink-0 font-mono text-faint">{MODE_LABEL[evaluation.mode]}</span>
              </li>
            ))}
          </ul>
        </details>
      ) : null}
    </div>
  );
}

// ------------------------------------------------------------------- patterns

function Patterns({ view, running }: { view: ViewState; running: boolean }) {
  const { run, agents, lastLlm } = view;
  const phases = run.active_agents.map((id) => agents[id]?.phase ?? "");
  const loop =
    phases.some((phase) => phase === "reflect" || phase === "security_fix" || phase === "revision") ||
    (run.phase === "implementation" && run.tdd?.phase === "green" && (run.tdd?.attempt ?? 1) > 1);
  const patterns: { icon: ReactNode; name: string; where: string; active: boolean }[] = [
    {
      icon: <Link2 />,
      name: "Chaînage de prompts",
      where: "Chaque phase consomme le livrable de la précédente : plan → architecture → tests → code → audit.",
      active: running,
    },
    {
      icon: <Route />,
      name: "Routage",
      where: "La demande est aiguillée selon son niveau, et chaque appel LLM vers le palier de modèle adapté.",
      active: running && (run.phase === "ingestion" || (lastLlm !== null && run.last_seq - lastLlm.seq <= 3)),
    },
    {
      icon: <Split />,
      name: "Parallélisation",
      where: "À l'audit, l'auditeur sécurité et le relecteur examinent le même changement en même temps.",
      active: running && run.active_agents.length > 1,
    },
    {
      icon: <GitFork />,
      name: "Orchestrateur-ouvriers",
      where: "L'orchestrateur distribue chaque tâche au rôle compétent et garde la main sur les verdicts.",
      active: running && run.active_agents.length > 0,
    },
    {
      icon: <Repeat />,
      name: "Évaluateur-optimiseur",
      where: "Les tests évaluent, le développeur optimise : la boucle se répète jusqu'au vert ou jusqu'à l'exception.",
      active: running && loop,
    },
  ];
  return (
    <ul className="space-y-1">
      {patterns.map((pattern) => (
        <li
          key={pattern.name}
          className={cn("flex items-start gap-2 rounded-md border px-2 py-1.5", pattern.active ? "border-accent/40 bg-accent/8" : "border-transparent")}
        >
          <span className={cn("mt-0.5 [&_svg]:size-3.5", pattern.active ? "text-accent" : "text-faint")}>{pattern.icon}</span>
          <span className="min-w-0 flex-1">
            <span className={cn("text-xs font-semibold", pattern.active ? "text-ink" : "text-dim")}>{pattern.name}</span>
            <span className="block text-[11px] leading-relaxed text-faint">{pattern.where}</span>
          </span>
          {pattern.active ? <Badge tone="accent">actif</Badge> : null}
        </li>
      ))}
    </ul>
  );
}

// -------------------------------------------------------------------- routing

function Routing({ view, meta, roleName }: { view: ViewState; meta: Meta; roleName: (id: string) => string }) {
  const { routing } = meta;
  const last = view.lastLlm;
  const kinds = Object.keys(routing.routes).sort((a, b) => {
    const order = (kind: string) => routing.escalation.indexOf(routing.routes[kind]);
    return order(b) - order(a) || a.localeCompare(b);
  });
  return (
    <div>
      <div className="overflow-hidden rounded-md border border-line">
        <table className="w-full border-collapse text-[11px]">
          <thead className="bg-raised text-faint">
            <tr>
              <th scope="col" className="px-2 py-1 text-left font-medium">Nature de la tâche</th>
              <th scope="col" className="px-2 py-1 text-left font-medium">Palier</th>
              <th scope="col" className="px-2 py-1 text-left font-medium">Modèle → repli</th>
            </tr>
          </thead>
          <tbody>
            {kinds.map((kind) => {
              const tier = routing.routes[kind];
              const models = routing.tiers[tier]?.models ?? [];
              const lit = last?.kind === kind;
              return (
                <tr key={kind} className={cn("border-t border-line", lit && "bg-accent/12")}>
                  <td className={cn("px-2 py-1", lit ? "font-semibold text-accent" : "text-dim")}>{KIND_LABEL[kind] ?? kind}</td>
                  <td className="px-2 py-1 font-mono text-dim" title={routing.tiers[tier]?.description}>{tier}</td>
                  <td className="px-2 py-1 font-mono text-faint">{models.map((model) => model.replace("omniforge-", "")).join(" → ")}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      <p className="mt-1.5 flex flex-wrap items-center gap-1 text-[10px] text-faint">
        Échelle d&apos;escalade après échecs :
        {routing.escalation.map((tier, index) => (
          <span key={tier} className="flex items-center gap-1">
            <span className="rounded bg-raised px-1 font-mono text-dim">{tier}</span>
            {index < routing.escalation.length - 1 ? <ArrowRight className="size-3" /> : null}
          </span>
        ))}
      </p>
      {last ? (
        <div className={cn("mt-2 rounded-md border bg-canvas p-2", last.ok ? "border-line" : "border-bad/40")}>
          <p className="text-[11px] text-faint">
            Dernier appel{last.role ? ` · ${roleName(last.role)}` : ""}
            {last.taskId ? ` · ${last.taskId}` : ""}
          </p>
          <p className="mt-0.5 font-mono text-[11px] text-ink">
            {last.kind} → {last.tier} → {last.model}
            {last.fallback_index > 0 ? <span className="text-warn"> (repli n°{last.fallback_index})</span> : null}
          </p>
          <p className="mt-0.5 text-[11px] leading-relaxed text-dim">{last.ok ? last.reason : `Échec : ${last.error ?? "erreur"}`}</p>
          {last.ok ? (
            <p className="mt-0.5 font-mono text-[10px] text-faint">
              {last.prompt_tokens ?? 0} + {last.completion_tokens ?? 0} tokens · {formatCost(last.cost_usd ?? 0)}
              {last.cost_estimated ? " estimé" : ""} · {last.latency_ms} ms · servi par {last.served_by}
            </p>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

// ---------------------------------------------------------------------- panel

function Section({ index, title, children }: { index: number; title: string; children: ReactNode }) {
  return (
    <section className="border-t border-line px-3 py-3 first:border-t-0">
      <h3 className="mb-2 flex items-center gap-2 text-xs font-semibold text-ink">
        <span className="flex size-4 items-center justify-center rounded-full bg-raised font-mono text-[10px] text-dim">{index}</span>
        {title}
      </h3>
      {children}
    </section>
  );
}

/** Explains the agentic method while it runs: every section is driven by live events. */
export function MethodPanel({
  view,
  status,
  meta,
  className,
}: {
  view: ViewState;
  status: RunStatus;
  meta: Meta;
  className?: string;
}) {
  const roleName = (id: string) => meta.roles.find((role) => role.id === id)?.name ?? id;
  const running = isActive(status);
  const now = narrate(view, status, roleName);

  return (
    <Panel className={cn("flex min-h-0 flex-col", className)}>
      <PanelHeader icon={<GraduationCap />} title="Méthode — ce qui se passe et pourquoi" />
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="border-b border-line bg-raised/50 px-3 py-2.5" role="status" aria-live="polite">
          <p className="text-[10px] font-semibold tracking-wider text-accent uppercase">En ce moment</p>
          <p className="mt-0.5 text-sm leading-snug font-medium text-ink">{now.headline}</p>
          <p className="mt-1 text-xs leading-relaxed text-dim">{now.detail}</p>
        </div>
        <Section index={1} title="Boucle Observer → Réfléchir → Agir">
          <OtaLoop view={view} running={running && view.run.active_agents.length > 0} roleName={roleName} />
        </Section>
        <Section index={2} title="Cycle TDD obligatoire">
          <TddCycle view={view} running={running && view.run.phase === "implementation"} />
        </Section>
        <Section index={3} title="Arbre de décision de la supervision">
          <DecisionTree view={view} meta={meta} />
        </Section>
        <Section index={4} title="Patterns d'orchestration">
          <Patterns view={view} running={running} />
        </Section>
        <Section index={5} title="Routage multi-modèles">
          <Routing view={view} meta={meta} roleName={roleName} />
        </Section>
      </div>
    </Panel>
  );
}
