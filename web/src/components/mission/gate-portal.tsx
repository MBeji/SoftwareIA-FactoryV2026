"use client";

import { Check, CirclePause, Mic, MicOff, Pencil, TriangleAlert, X } from "lucide-react";
import { useCallback, useState, type ReactNode } from "react";

import { FindingsList } from "@/components/findings";
import { Markdown } from "@/components/markdown";
import { PullRequestView } from "@/components/mission/deliverable";
import { PlanView } from "@/components/plan-view";
import { ConfidenceBar, MODE_TONE } from "@/components/status";
import { Badge } from "@/components/ui/badge";
import { Button, type ButtonProps } from "@/components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Label, Textarea } from "@/components/ui/field";
import { api, ApiError } from "@/lib/api";
import { EXCEPTION_LABEL, MODE_LABEL, STAGE_LABEL, formatCost } from "@/lib/format";
import type { GateDecision, GateView, Proposal, TaskNode } from "@/lib/types";
import { useSpeechInput } from "@/lib/use-speech-input";
import { cn } from "@/lib/utils";

const DECISION: Record<GateDecision, { variant: ButtonProps["variant"]; icon: ReactNode }> = {
  approve: { variant: "success", icon: <Check /> },
  request_changes: { variant: "warning", icon: <Pencil /> },
  reject: { variant: "dangerOutline", icon: <X /> },
};

function Output({ title, text }: { title: string; text: string }) {
  return (
    <div>
      <p className="mb-1 text-[11px] font-semibold tracking-wide text-dim uppercase">{title}</p>
      <pre className="max-h-64 overflow-auto rounded-md border border-line bg-canvas p-2.5 font-mono text-[11px] leading-[17px] whitespace-pre-wrap text-dim">
        {text.trim() || "(vide)"}
      </pre>
    </div>
  );
}

function TaskSummary({ task }: { task: TaskNode }) {
  return (
    <div className="rounded-md border border-line bg-raised/40 p-3">
      <p className="text-sm font-medium text-ink">
        <span className="mr-1.5 font-mono text-xs text-faint">{task.id}</span>
        {task.title}
      </p>
      <ul className="mt-1.5 list-disc space-y-0.5 pl-4 text-xs text-dim">
        {task.acceptance_criteria.map((criterion, index) => (
          <li key={index}>{criterion}</li>
        ))}
      </ul>
    </div>
  );
}

/** Renders what the agents propose, by proposal kind. */
function ProposalView({ proposal, threshold }: { proposal: Proposal; threshold: number }) {
  switch (proposal.kind) {
    case "plan":
      return <PlanView plan={proposal.plan} compact />;
    case "architecture":
      return <Markdown source={proposal.document} />;
    case "test_plan":
      return (
        <div className="space-y-3">
          <TaskSummary task={proposal.task} />
          <div className="rounded-md border border-bad/30 bg-bad/8 px-3 py-2 text-xs">
            <span className="font-semibold text-bad">Exécution RED</span>
            <span className="text-dim"> — les tests doivent échouer avant toute implémentation : </span>
            <span className="text-ink">{proposal.red_run.summary}</span>
          </div>
          {proposal.files.map((file) => (
            <Output key={file.path} title={file.path} text={file.content} />
          ))}
          <Output title="Sortie de l'exécution RED" text={proposal.red_run.output} />
        </div>
      );
    case "tdd_divergence":
      return (
        <div className="space-y-3">
          <TaskSummary task={proposal.task} />
          <p className="rounded-md border border-warn/30 bg-warn/8 px-3 py-2 text-xs text-ink">
            <span className="font-semibold text-warn">{proposal.attempts} tentative(s) GREEN</span> sans faire passer les tests : {proposal.last_run.summary}
          </p>
          {proposal.last_reflection ? (
            <div className="rounded-md border border-line bg-raised/40 p-3 text-xs">
              <p className="font-semibold text-ink">Dernier diagnostic de la boucle d&apos;auto-correction</p>
              <p className="mt-1 leading-relaxed text-dim">{proposal.last_reflection.root_cause}</p>
              {proposal.last_reflection.fix_plan ? (
                <p className="mt-1 leading-relaxed text-dim">Correctif proposé : {proposal.last_reflection.fix_plan}</p>
              ) : null}
            </div>
          ) : null}
          <Output title="Dernière exécution des tests" text={proposal.last_run.output} />
        </div>
      );
    case "security":
      return <FindingsList findings={proposal.findings} />;
    case "pull_request":
      return <PullRequestView pullRequest={proposal} threshold={threshold} />;
    case "budget":
      return (
        <dl className="grid grid-cols-3 gap-2 text-center">
          {[
            ["Dépensé", proposal.spent_usd],
            ["Plafond actuel", proposal.limit_usd],
            ["Plafond proposé", proposal.proposed_limit_usd],
          ].map(([label, amount]) => (
            <div key={label} className="rounded-md border border-line bg-raised/40 p-3">
              <dd className="font-mono text-lg text-ink">{formatCost(Number(amount))}</dd>
              <dt className="text-[11px] text-faint">{label}</dt>
            </div>
          ))}
        </dl>
      );
    default:
      return <Output title="Proposition" text={JSON.stringify(proposal, null, 2)} />;
  }
}

/**
 * Validation portal: presents a pending gate, explains why it was raised and collects
 * the supervisor's decision with written or dictated feedback.
 */
export function GatePortal({
  runId,
  gate,
  open,
  onOpenChange,
}: {
  runId: string;
  gate: GateView;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const request = gate.request;
  const [feedback, setFeedback] = useState("");
  const [busy, setBusy] = useState<GateDecision | null>(null);
  const [error, setError] = useState<string | null>(null);
  const append = useCallback((text: string) => {
    if (text) setFeedback((current) => (current.trim() ? `${current.trimEnd()} ${text}` : text));
  }, []);
  const speech = useSpeechInput(append);

  const decide = async (decision: GateDecision) => {
    speech.stop();
    setBusy(decision);
    setError(null);
    try {
      await api.resolveGate(runId, gate.gate_id, decision, feedback.trim() || null, "superviseur (IHM)");
      // The dialog closes when the `gate.resolved` event arrives on the stream.
    } catch (reason) {
      setBusy(null);
      setError(
        reason instanceof ApiError && (reason.status === 404 || reason.status === 409)
          ? "Cette validation n'est plus en attente (déjà traitée ou run arrêté)."
          : reason instanceof Error
            ? reason.message
            : "Erreur inconnue",
      );
    }
  };

  const needsFeedback = feedback.trim().length === 0;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent size="xl" aria-describedby="gate-summary">
        <DialogHeader>
          <p className="flex flex-wrap items-center gap-1.5 text-[11px] font-semibold tracking-wider text-warn uppercase">
            <CirclePause className="size-3.5" /> Portail de validation
            <Badge tone={MODE_TONE[request.mode]}>{MODE_LABEL[request.mode]}</Badge>
            <Badge>{STAGE_LABEL[request.stage]}</Badge>
            {request.task_id ? <Badge>{request.task_id}</Badge> : null}
          </p>
          <DialogTitle className="mt-1.5 text-lg">{request.title}</DialogTitle>
          <DialogDescription id="gate-summary">{request.summary}</DialogDescription>
        </DialogHeader>

        <DialogBody className="space-y-4">
          <section className="rounded-md border border-warn/30 bg-warn/8 p-3" aria-label="Pourquoi cette validation">
            <p className="text-xs font-semibold text-warn">Pourquoi votre décision est requise</p>
            <p className="mt-1 text-sm text-ink">{request.reason}</p>
            <div className="mt-2 flex max-w-md items-center gap-2 text-[11px] text-dim">
              Confiance de l&apos;agent
              <ConfidenceBar value={request.confidence} threshold={request.threshold} className="flex-1" />
            </div>
            {request.exceptions.length > 0 ? (
              <ul className="mt-2 space-y-1">
                {request.exceptions.map((exception, index) => (
                  <li key={index} className="flex items-start gap-1.5 text-xs text-ink">
                    <Badge tone="warn" className="shrink-0">
                      {EXCEPTION_LABEL[exception.kind] ?? exception.kind}
                    </Badge>
                    {exception.detail}
                  </li>
                ))}
              </ul>
            ) : null}
          </section>

          <section aria-label="Proposition des agents">
            <p className="mb-2 text-[11px] font-semibold tracking-wide text-dim uppercase">Proposition soumise</p>
            <ProposalView proposal={request.proposal} threshold={request.threshold} />
          </section>

          <section>
            <div className="mb-1 flex items-center justify-between gap-2">
              <Label htmlFor="gate-feedback" className="mb-0">
                Votre retour <span className="font-normal text-faint">— transmis aux agents ; requis pour demander des modifications</span>
              </Label>
              <Button
                size="sm"
                variant={speech.listening ? "danger" : "secondary"}
                disabled={!speech.supported}
                onClick={speech.toggle}
                aria-pressed={speech.listening}
                title={speech.supported ? "Dicter le retour (français)" : "Dictée vocale non prise en charge par ce navigateur"}
              >
                {speech.listening ? <MicOff /> : <Mic />}
                {speech.listening ? "Arrêter la dictée" : "Dicter"}
              </Button>
            </div>
            <Textarea
              id="gate-feedback"
              rows={3}
              value={feedback}
              onChange={(event) => setFeedback(event.target.value)}
              placeholder="Ex. : ajoute un critère sur l'expiration des jetons ; sépare le module de facturation…"
            />
            <p className="mt-1 min-h-4 text-[11px]" aria-live="polite">
              {speech.listening ? (
                <span className="text-accent">
                  <span className="mr-1 inline-block size-1.5 animate-pulse rounded-full bg-bad align-middle" />
                  Écoute en cours… {speech.interim ? <span className="text-dim italic">{speech.interim}</span> : null}
                </span>
              ) : speech.error ? (
                <span className="text-warn">{speech.error}</span>
              ) : !speech.supported ? (
                <span className="text-faint">
                  La dictée vocale n&apos;est pas prise en charge par ce navigateur (disponible sur Chrome et Edge) : utilisez le champ texte.
                </span>
              ) : null}
            </p>
          </section>

          {error ? (
            <p role="alert" className="flex items-start gap-2 rounded-md border border-bad/40 bg-bad/10 px-3 py-2 text-sm text-bad">
              <TriangleAlert className="mt-0.5 size-4 shrink-0" /> {error}
            </p>
          ) : null}
        </DialogBody>

        <DialogFooter>
          <span className="mr-auto text-[11px] text-faint">Fermer ne décide rien : le run attend.</span>
          {[...request.options].reverse().map((option) => {
            const style = DECISION[option.decision];
            const blocked = option.decision === "request_changes" && needsFeedback;
            return (
              <Button
                key={option.decision}
                variant={style.variant}
                disabled={busy !== null || blocked}
                title={blocked ? "Précisez les modifications attendues dans le champ de retour" : undefined}
                onClick={() => void decide(option.decision)}
                className={cn(option.decision === "approve" && "order-last")}
              >
                {style.icon}
                {busy === option.decision ? "Envoi…" : option.label}
              </Button>
            );
          })}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
