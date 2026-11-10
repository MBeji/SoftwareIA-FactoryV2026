"use client";

import { ChevronRight, SquareTerminal, TriangleAlert } from "lucide-react";
import { useEffect, useRef, useState, type FormEvent } from "react";

import { Badge, type BadgeTone } from "@/components/ui/badge";
import { EmptyState } from "@/components/ui/panel";
import { api, ApiError } from "@/lib/api";
import { formatTime } from "@/lib/format";
import type { TerminalEntry } from "@/lib/reducer";
import type { Meta } from "@/lib/types";
import { cn } from "@/lib/utils";

const CHANNEL: Record<TerminalEntry["channel"], { label: string; tone: BadgeTone }> = {
  shell: { label: "shell", tone: "neutral" },
  tests: { label: "tests", tone: "info" },
  git: { label: "git", tone: "neutral" },
  search: { label: "recherche", tone: "neutral" },
  verdict: { label: "verdict", tone: "violet" },
};

function Entry({ entry, author }: { entry: TerminalEntry; author: string }) {
  const failed = entry.ok === false;
  return (
    <div className="border-b border-line/60 px-3 py-1.5 last:border-b-0">
      <div className="flex flex-wrap items-center gap-1.5 text-[10px] text-faint">
        <span className="font-mono">{formatTime(entry.ts)}</span>
        <span className={cn(entry.by === "user" ? "text-warn" : entry.by === "orchestrator" ? "text-accent" : "text-dim")}>{author}</span>
        <Badge tone={CHANNEL[entry.channel].tone}>{CHANNEL[entry.channel].label}</Badge>
        {entry.taskId ? <span className="font-mono">{entry.taskId}</span> : null}
        {entry.pending ? (
          <span className="animate-pulse text-accent">en cours…</span>
        ) : entry.exitCode !== null ? (
          <span className={cn("font-mono", failed ? "text-bad" : "text-ok")}>code {entry.exitCode}</span>
        ) : entry.ok !== null ? (
          <span className={failed ? "text-bad" : "text-ok"}>{failed ? "échec" : "ok"}</span>
        ) : null}
      </div>
      <p className="mt-0.5 font-mono text-xs break-all whitespace-pre-wrap text-ink">
        <span className="text-ok select-none">$ </span>
        {entry.command}
      </p>
      {entry.output ? (
        <pre className={cn("mt-0.5 max-h-56 overflow-auto font-mono text-[11px] leading-[17px] whitespace-pre-wrap", failed ? "text-bad/90" : "text-dim")}>
          {entry.output}
        </pre>
      ) : null}
    </div>
  );
}

/**
 * Sandbox terminal: every command run by agents, by the orchestrator (tests, lint, git)
 * and by the human operator, plus a prompt to run commands while the run is active.
 */
export function TerminalPane({
  runId,
  entries,
  active,
  meta,
}: {
  runId: string;
  entries: TerminalEntry[];
  active: boolean;
  meta: Meta | null;
}) {
  const scroller = useRef<HTMLDivElement>(null);
  const follow = useRef(true);
  const [command, setCommand] = useState("");
  const [history, setHistory] = useState<string[]>([]);
  const [cursor, setCursor] = useState(-1);
  const [busy, setBusy] = useState(false);
  const [refusal, setRefusal] = useState<string | null>(null);

  // Stick to the bottom unless the operator scrolled up to read something.
  useEffect(() => {
    const element = scroller.current;
    if (element && follow.current) element.scrollTop = element.scrollHeight;
  }, [entries]);

  const author = (by: string): string =>
    by === "user" ? "vous" : by === "orchestrator" ? "orchestrateur" : (meta?.roles.find((role) => role.id === by)?.name ?? by);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const line = command.trim();
    if (!line || busy) return;
    setBusy(true);
    setRefusal(null);
    follow.current = true;
    try {
      await api.terminal(runId, line); // the output arrives through the event stream
      setHistory((previous) => [...previous.filter((item) => item !== line), line].slice(-50));
      setCommand("");
      setCursor(-1);
    } catch (reason) {
      if (reason instanceof ApiError && reason.status === 403) {
        setRefusal(`Commande refusée par la politique de sécurité — ${reason.message.replace(/^policy violation:\s*/, "")}`);
      } else if (reason instanceof ApiError && reason.status === 409) {
        setRefusal("Le run n'est plus actif : la sandbox est arrêtée.");
      } else {
        setRefusal(reason instanceof Error ? reason.message : "Erreur inconnue");
      }
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex h-full min-h-0 flex-col bg-canvas">
      <div
        ref={scroller}
        onScroll={(event) => {
          const element = event.currentTarget;
          follow.current = element.scrollHeight - element.scrollTop - element.clientHeight < 40;
        }}
        className="min-h-0 flex-1 overflow-y-auto"
        role="log"
        aria-label="Sortie du terminal de la sandbox"
      >
        {entries.length === 0 ? (
          <EmptyState icon={<SquareTerminal />} title="Aucune commande pour l'instant">
            Les commandes des agents, les tests et les opérations git de l&apos;orchestrateur s&apos;affichent ici en direct.
          </EmptyState>
        ) : (
          entries.map((entry) => <Entry key={entry.key} entry={entry} author={author(entry.by)} />)
        )}
      </div>
      {refusal ? (
        <p role="alert" className="flex items-start gap-1.5 border-t border-bad/40 bg-bad/10 px-3 py-1.5 text-xs text-bad">
          <TriangleAlert className="mt-0.5 size-3.5 shrink-0" /> {refusal}
        </p>
      ) : null}
      <form onSubmit={submit} className="flex shrink-0 items-center gap-2 border-t border-line bg-panel px-3 py-1.5">
        <ChevronRight className={cn("size-4 shrink-0", active ? "text-ok" : "text-faint")} />
        <label htmlFor="terminal-input" className="sr-only">
          Commande à exécuter dans la sandbox
        </label>
        <input
          id="terminal-input"
          value={command}
          disabled={!active || busy}
          autoComplete="off"
          spellCheck={false}
          onChange={(event) => setCommand(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "ArrowUp" && history.length > 0) {
              event.preventDefault();
              const next = cursor < 0 ? history.length - 1 : Math.max(0, cursor - 1);
              setCursor(next);
              setCommand(history[next]);
            } else if (event.key === "ArrowDown" && cursor >= 0) {
              event.preventDefault();
              const next = cursor + 1;
              setCursor(next >= history.length ? -1 : next);
              setCommand(next >= history.length ? "" : history[next]);
            }
          }}
          placeholder={
            active
              ? "Exécuter une commande dans la sandbox (ex. git log --oneline, ls, pytest -q)…"
              : "Terminal en lecture seule : le run n'est plus actif"
          }
          className="h-7 min-w-0 flex-1 bg-transparent font-mono text-xs text-ink placeholder:text-faint focus:outline-none disabled:opacity-60"
        />
        {busy ? <span className="text-[11px] text-faint">exécution…</span> : null}
      </form>
    </div>
  );
}
