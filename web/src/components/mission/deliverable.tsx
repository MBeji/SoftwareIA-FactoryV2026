"use client";

import { GitBranch, GitCommitHorizontal, GitPullRequest, PencilRuler, ShieldAlert, TriangleAlert } from "lucide-react";
import type { ReactNode } from "react";

import { DiffView } from "@/components/diff-view";
import { FindingsList } from "@/components/findings";
import { Markdown } from "@/components/markdown";
import { ConfidenceBar } from "@/components/status";
import { Badge } from "@/components/ui/badge";
import { EmptyState } from "@/components/ui/panel";
import type { ViewState } from "@/lib/reducer";
import type { ArchitectureArtifact, PullRequest, RunStatus } from "@/lib/types";

function Block({ icon, title, aside, children }: { icon: ReactNode; title: string; aside?: ReactNode; children: ReactNode }) {
  return (
    <section className="rounded-lg border border-line bg-raised/30">
      <div className="flex flex-wrap items-center justify-between gap-2 border-b border-line px-3 py-2">
        <h3 className="flex items-center gap-2 text-sm font-semibold text-ink [&_svg]:size-4 [&_svg]:text-dim">
          {icon}
          {title}
        </h3>
        {aside}
      </div>
      <div className="p-3">{children}</div>
    </section>
  );
}

export function PullRequestView({ pullRequest, threshold }: { pullRequest: PullRequest; threshold?: number }) {
  return (
    <div className="space-y-3">
      <div>
        <p className="text-base font-semibold text-ink">{pullRequest.title}</p>
        <p className="mt-1 flex flex-wrap items-center gap-1.5 font-mono text-[11px] text-dim">
          <GitBranch className="size-3.5 text-faint" />
          <span className="rounded bg-canvas px-1.5 py-0.5">{pullRequest.branch}</span>→
          <span className="rounded bg-canvas px-1.5 py-0.5">{pullRequest.base}</span>
        </p>
        <div className="mt-2 flex max-w-sm items-center gap-2 text-[11px] text-faint">
          Confiance globale <ConfidenceBar value={pullRequest.confidence} threshold={threshold} className="flex-1" />
        </div>
      </div>
      <Markdown source={pullRequest.body} />
      <div className="grid gap-3 lg:grid-cols-2">
        <div>
          <p className="mb-1 text-[11px] font-semibold tracking-wide text-dim uppercase">Commits ({pullRequest.commits.length})</p>
          <ul className="space-y-0.5 rounded-md border border-line bg-canvas p-2 font-mono text-[11px] text-ink">
            {pullRequest.commits.map((commit) => (
              <li key={commit} className="flex items-start gap-1.5">
                <GitCommitHorizontal className="mt-0.5 size-3 shrink-0 text-faint" />
                <span className="break-all">{commit}</span>
              </li>
            ))}
          </ul>
        </div>
        <div>
          <p className="mb-1 text-[11px] font-semibold tracking-wide text-dim uppercase">Fichiers modifiés</p>
          <pre className="overflow-x-auto rounded-md border border-line bg-canvas p-2 font-mono text-[11px] leading-[17px] text-dim">
            {pullRequest.diff_stat.trim() || "—"}
          </pre>
        </div>
      </div>
      <details>
        <summary className="cursor-pointer text-xs text-accent hover:underline">Afficher le diff complet</summary>
        <div className="mt-2">
          <DiffView diff={pullRequest.diff} />
        </div>
      </details>
    </div>
  );
}

/** What the run produces: merge request, architecture, security report, policy violations. */
export function Deliverable({ view, status }: { view: ViewState; status: RunStatus }) {
  const { run } = view;
  const pullRequest = run.artifacts.find((artifact): artifact is PullRequest => artifact.kind === "pull_request");
  const architecture = run.artifacts.find((artifact): artifact is ArchitectureArtifact => artifact.kind === "architecture");
  const pendingMerge = run.gates.find((gate) => !gate.resolution && gate.request.proposal.kind === "pull_request");

  if (!pullRequest && !architecture && run.findings.length === 0 && run.violations.length === 0) {
    return (
      <EmptyState icon={<GitPullRequest />} title="Rien à livrer pour l'instant">
        L&apos;architecture, le rapport de sécurité puis la merge request apparaissent ici au fil du run.
      </EmptyState>
    );
  }

  return (
    <div className="h-full space-y-3 overflow-y-auto p-3">
      <Block
        icon={<GitPullRequest />}
        title="Merge request"
        aside={
          pullRequest ? (
            <Badge tone="ok">prête</Badge>
          ) : pendingMerge ? (
            <Badge tone="warn">en attente de validation</Badge>
          ) : (
            <Badge>{status === "failed" || status === "cancelled" ? "non produite" : "à venir"}</Badge>
          )
        }
      >
        {pullRequest ? (
          <PullRequestView pullRequest={pullRequest} threshold={run.supervision.confidence_threshold} />
        ) : pendingMerge && pendingMerge.request.proposal.kind === "pull_request" ? (
          <PullRequestView pullRequest={pendingMerge.request.proposal} threshold={run.supervision.confidence_threshold} />
        ) : (
          <p className="text-xs text-faint">
            La merge request est produite à la phase de livraison, après une dernière exécution complète de la suite de tests.
          </p>
        )}
      </Block>

      {architecture ? (
        <Block icon={<PencilRuler />} title="Architecture" aside={<span className="font-mono text-[11px] text-faint">{architecture.path} · révision {architecture.revision}</span>}>
          <Markdown source={architecture.content} />
        </Block>
      ) : null}

      <Block icon={<ShieldAlert />} title="Sécurité" aside={<span className="text-[11px] text-faint">scanner statique déterministe + auditeur</span>}>
        <FindingsList
          findings={run.findings}
          emptyLabel={run.phase === "audit" || run.phase === "delivery" || run.phase === "done" ? undefined : "L'audit de sécurité n'a pas encore eu lieu."}
        />
      </Block>

      {run.violations.length > 0 ? (
        <Block icon={<TriangleAlert />} title={`Violations de politique (${run.violations.length})`}>
          <ul className="space-y-1.5">
            {run.violations.map((violation, index) => (
              <li key={index} className="rounded-md border border-bad/30 bg-bad/8 px-2.5 py-1.5 text-xs">
                <span className="font-mono text-bad">{violation.rule}</span>
                <span className="text-ink"> — {violation.detail}</span>
              </li>
            ))}
          </ul>
        </Block>
      ) : null}
    </div>
  );
}
