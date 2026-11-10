"use client";

import { ScrollText } from "lucide-react";
import { useMemo, useState } from "react";

import { Badge, type BadgeTone } from "@/components/ui/badge";
import { Input, Select } from "@/components/ui/field";
import { EmptyState } from "@/components/ui/panel";
import {
  DECISION_LABEL,
  MODE_LABEL,
  PHASE_LABEL,
  STATUS_LABEL,
  TASK_STATUS_LABEL,
  VERDICT_LABEL,
  formatTime,
  roleFromSource,
  truncate,
} from "@/lib/format";
import { describeAction } from "@/lib/reducer";
import type { FactoryEvent, Json, JsonObject, Meta } from "@/lib/types";
import { cn } from "@/lib/utils";

const GROUPS: { id: string; label: string; tone: BadgeTone; types: string[] }[] = [
  { id: "run", label: "Run", tone: "neutral", types: ["run.created", "run.status_changed", "run.phase_changed", "ingestion.classified", "plan.created", "task.status_changed", "artifact.created", "log"] },
  { id: "agent", label: "Agents", tone: "accent", types: ["agent.turn_started", "agent.thought", "agent.turn_ended"] },
  { id: "tool", label: "Outils", tone: "info", types: ["action", "observation", "workspace.file_changed", "terminal.output"] },
  { id: "tdd", label: "TDD", tone: "ok", types: ["tdd.phase_changed", "tdd.test_result", "tdd.reflection"] },
  { id: "gate", label: "Supervision", tone: "warn", types: ["gate.evaluated", "gate.requested", "gate.resolved", "supervision.changed", "policy.violation", "security.findings"] },
  { id: "llm", label: "LLM", tone: "violet", types: ["llm.call"] },
];

const groupOf = (type: string) => GROUPS.find((group) => group.types.includes(type)) ?? GROUPS[0];
const s = (value: Json | undefined): string => (typeof value === "string" ? value : value === undefined || value === null ? "" : String(value));
const o = (value: Json | undefined): JsonObject => (value && typeof value === "object" && !Array.isArray(value) ? value : {});
/** French label for a backend enum value, falling back to the raw value. */
const label = (table: Record<string, string>, value: Json | undefined): string => table[s(value)] ?? s(value);

/** One-line French summary of an event. */
function summarize(event: FactoryEvent): string {
  const p = event.payload;
  switch (event.type) {
    case "run.created":
      return `Run créé : ${s(p.title)}`;
    case "run.status_changed":
      return `Statut → ${label(STATUS_LABEL, p.status)}${p.error ? ` — ${s(p.error)}` : ""}`;
    case "run.phase_changed":
      return `Phase → ${label(PHASE_LABEL, p.phase)}`;
    case "ingestion.classified":
      return `Demande classée niveau ${s(p.level)} (${s(p.label)}) : ${s(p.rationale)}`;
    case "plan.created": {
      const plan = o(p.plan);
      const order = Array.isArray(plan.execution_order) ? plan.execution_order.length : 0;
      return `Plan (révision ${s(p.revision)}) : ${order} tâche(s), ${s(plan.total_points)} point(s)`;
    }
    case "task.status_changed":
      return `Tâche → ${label(TASK_STATUS_LABEL, p.status)}${p.attempts ? ` (${s(p.attempts)} tentative(s) GREEN)` : ""}`;
    case "agent.turn_started":
      return `${s(p.name)} prend la parole — ${s(p.phase)}${Number(p.attempt) > 1 ? ` (tentative ${s(p.attempt)})` : ""}`;
    case "agent.thought":
      return `« ${s(p.text)} »`;
    case "agent.turn_ended":
      return p.ok === true ? `Rend la main : ${s(p.summary)}` : `Tour en échec : ${s(p.error)}`;
    case "action":
      return `→ ${describeAction(s(p.tool), o(p.args))}`;
    case "observation":
      return `← ${p.ok === true ? "ok" : "erreur"} · ${s(p.output).split("\n")[0]}`;
    case "workspace.file_changed":
      return `${p.created === true ? "Création" : "Modification"} de ${s(p.path)} (+${s(p.additions)} −${s(p.deletions)})`;
    case "tdd.phase_changed":
      return `TDD → ${s(p.phase).toUpperCase()} (tentative ${s(p.attempt)}/${s(p.max_attempts)})`;
    case "tdd.test_result":
      return `Tests [${s(p.phase)}, attendu : ${s(p.expected) === "fail" ? "échec" : "succès"}] ${s(p.summary)} ${p.as_expected === true ? "✓" : "✗"}`;
    case "tdd.reflection":
      return `Diagnostic : ${s(p.root_cause)}`;
    case "llm.call":
      return p.ok === true
        ? `${s(p.kind)} → ${s(p.tier)} → ${s(p.model)} · ${s(p.prompt_tokens)}+${s(p.completion_tokens)} tokens`
        : `Appel en échec sur ${s(p.model)} : ${s(p.error)}`;
    case "gate.evaluated": {
      const evaluation = o(p.evaluation);
      return `${s(p.title)} [${label(MODE_LABEL, evaluation.mode)}] → ${label(VERDICT_LABEL, evaluation.verdict)} — ${s(evaluation.reason)}`;
    }
    case "gate.requested":
      return `Validation requise : ${s(p.title)} — ${s(p.reason)}`;
    case "gate.resolved":
      return `Décision : ${label(DECISION_LABEL, p.decision)} par ${s(p.resolver)}${p.feedback ? ` — « ${s(p.feedback)} »` : ""}`;
    case "supervision.changed":
      return `Politique de supervision mise à jour (mode ${label(MODE_LABEL, o(p.policy).default_mode)}) par ${s(p.by)}`;
    case "policy.violation":
      return `Violation de politique [${s(p.rule)}] ${s(p.detail)}`;
    case "security.findings":
      return `Sécurité (tour ${s(p.round)}) : ${Array.isArray(p.findings) ? p.findings.length : 0} finding(s), ${s(p.blocking)} bloquant(s)`;
    case "artifact.created":
      return `Artefact : ${s(p.title) || s(p.kind)}`;
    case "terminal.output":
      return `$ ${s(p.command)} (code ${s(p.exit_code)})`;
    case "log":
      return s(p.message);
    default:
      return event.type;
  }
}

const PAGE = 300;

/** Filterable audit trail of the run: every event, newest first. */
export function Journal({ events, meta }: { events: FactoryEvent[]; meta: Meta | null }) {
  const [group, setGroup] = useState("all");
  const [source, setSource] = useState("all");
  const [task, setTask] = useState("all");
  const [query, setQuery] = useState("");
  const [limit, setLimit] = useState(PAGE);

  const sources = useMemo(() => Array.from(new Set(events.map((event) => event.source))).sort(), [events]);
  const tasks = useMemo(
    () => Array.from(new Set(events.map((event) => event.task_id).filter((id): id is string => Boolean(id)))).sort(),
    [events],
  );
  const sourceLabel = (value: string): string => {
    const role = roleFromSource(value);
    if (role) return meta?.roles.find((candidate) => candidate.id === role)?.name ?? role;
    return { orchestrator: "Orchestrateur", gatekeeper: "Supervision", user: "Utilisateur" }[value] ?? value;
  };

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    const rows: { event: FactoryEvent; text: string }[] = [];
    for (let index = events.length - 1; index >= 0; index -= 1) {
      const event = events[index];
      if (group !== "all" && groupOf(event.type).id !== group) continue;
      if (source !== "all" && event.source !== source) continue;
      if (task !== "all" && event.task_id !== task) continue;
      const text = summarize(event);
      if (needle && !text.toLowerCase().includes(needle) && !event.type.includes(needle)) continue;
      rows.push({ event, text });
    }
    return rows;
  }, [events, group, query, source, task]);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-line px-3 py-2">
        <label className="sr-only" htmlFor="journal-group">Type d&apos;événement</label>
        <Select id="journal-group" value={group} onChange={(event) => setGroup(event.target.value)} className="h-7 w-36 text-xs">
          <option value="all">Tous les types</option>
          {GROUPS.map((candidate) => (
            <option key={candidate.id} value={candidate.id}>{candidate.label}</option>
          ))}
        </Select>
        <label className="sr-only" htmlFor="journal-source">Émetteur</label>
        <Select id="journal-source" value={source} onChange={(event) => setSource(event.target.value)} className="h-7 w-44 text-xs">
          <option value="all">Tous les émetteurs</option>
          {sources.map((candidate) => (
            <option key={candidate} value={candidate}>{sourceLabel(candidate)}</option>
          ))}
        </Select>
        <label className="sr-only" htmlFor="journal-task">Tâche</label>
        <Select id="journal-task" value={task} onChange={(event) => setTask(event.target.value)} className="h-7 w-40 text-xs">
          <option value="all">Toutes les tâches</option>
          {tasks.map((candidate) => (
            <option key={candidate} value={candidate}>{candidate}</option>
          ))}
        </Select>
        <label className="sr-only" htmlFor="journal-query">Rechercher dans le journal</label>
        <Input id="journal-query" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Rechercher…" className="h-7 min-w-32 flex-1 text-xs" />
        <span className="font-mono text-[11px] text-faint">{filtered.length} / {events.length}</span>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {filtered.length === 0 ? (
          <EmptyState icon={<ScrollText />} title={events.length === 0 ? "Aucun événement" : "Aucun événement ne correspond aux filtres"} />
        ) : (
          <ol>
            {filtered.slice(0, limit).map(({ event, text }) => {
              const kind = groupOf(event.type);
              return (
                <li key={event.seq} className="flex items-start gap-2 border-b border-line/60 px-3 py-1 text-xs hover:bg-hover/50">
                  <span className="w-9 shrink-0 text-right font-mono text-[10px] leading-5 text-faint">#{event.seq}</span>
                  <span className="w-16 shrink-0 font-mono text-[10px] leading-5 text-faint">{formatTime(event.ts)}</span>
                  <Badge tone={kind.tone} className="mt-0.5 w-20 shrink-0 justify-center">{kind.label}</Badge>
                  <span className="w-32 shrink-0 truncate leading-5 text-dim" title={event.source}>{sourceLabel(event.source)}</span>
                  {event.task_id ? <span className="shrink-0 font-mono text-[10px] leading-5 text-faint">{event.task_id}</span> : null}
                  <span className={cn("min-w-0 flex-1 leading-5 break-words", event.type === "policy.violation" ? "text-bad" : "text-ink")} title={event.type}>
                    {truncate(text, 260)}
                  </span>
                </li>
              );
            })}
          </ol>
        )}
        {filtered.length > limit ? (
          <button type="button" onClick={() => setLimit((value) => value + PAGE)} className="w-full py-2 text-xs text-accent hover:bg-hover">
            Afficher {Math.min(PAGE, filtered.length - limit)} événements de plus
          </button>
        ) : null}
      </div>
    </div>
  );
}
