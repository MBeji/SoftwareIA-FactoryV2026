import { ShieldCheck } from "lucide-react";

import { SEVERITY_TONE } from "@/components/status";
import { Badge } from "@/components/ui/badge";
import { SEVERITY_LABEL } from "@/lib/format";
import type { Finding } from "@/lib/types";

export function FindingsList({ findings, emptyLabel }: { findings: Finding[]; emptyLabel?: string }) {
  if (findings.length === 0) {
    return (
      <p className="flex items-center gap-2 rounded-md border border-ok/30 bg-ok/8 px-3 py-2 text-xs text-ok">
        <ShieldCheck className="size-4" /> {emptyLabel ?? "Aucun finding de sécurité ouvert."}
      </p>
    );
  }
  return (
    <ul className="space-y-1.5">
      {findings.map((finding, index) => (
        <li key={`${finding.rule_id}-${finding.path}-${finding.line}-${index}`} className="rounded-md border border-line bg-raised/40 p-2.5">
          <div className="flex flex-wrap items-center gap-1.5">
            <Badge tone={SEVERITY_TONE[finding.severity]}>{SEVERITY_LABEL[finding.severity]}</Badge>
            <span className="font-mono text-[11px] text-faint">{finding.rule_id}</span>
            <span className="font-mono text-[11px] text-dim">
              {finding.path}
              {finding.line ? `:${finding.line}` : ""}
            </span>
          </div>
          <p className="mt-1 text-sm text-ink">{finding.message}</p>
          {finding.snippet ? (
            <pre className="mt-1 overflow-x-auto rounded bg-canvas px-2 py-1 font-mono text-[11px] text-bad/90">{finding.snippet}</pre>
          ) : null}
          {finding.remediation ? <p className="mt-1 text-xs text-dim">Remédiation : {finding.remediation}</p> : null}
        </li>
      ))}
    </ul>
  );
}
