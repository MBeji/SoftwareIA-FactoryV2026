"use client";

import { Bot, Eye, Hand } from "lucide-react";
import { useId, type ReactNode } from "react";

import { Input, Label, Select } from "@/components/ui/field";
import { MODE_LABEL, MODE_LONG, SEVERITY_LABEL, STAGE_HINT, STAGE_LABEL, formatPercent } from "@/lib/format";
import type { Meta, Severity, Stage, SupervisionMode, SupervisionPolicy } from "@/lib/types";
import { cn } from "@/lib/utils";

const MODES: { id: SupervisionMode; icon: ReactNode; accent: string }[] = [
  { id: "hitl", icon: <Hand />, accent: "border-warn text-warn" },
  { id: "hotl", icon: <Eye />, accent: "border-accent text-accent" },
  { id: "autonomous", icon: <Bot />, accent: "border-violet text-violet" },
];

const SEVERITIES: Severity[] = ["low", "medium", "high", "critical"];

type Override = SupervisionMode | "inherit";

/**
 * Governance "à la carte": default mode, per-stage overrides and exception thresholds.
 * Emits a `SupervisionPolicy` exactly as the backend expects it.
 */
export function SupervisionSelector({
  value,
  onChange,
  stages,
  disabled,
  dense,
  collapsible,
}: {
  value: SupervisionPolicy;
  onChange: (policy: SupervisionPolicy) => void;
  stages: Meta["stages"];
  disabled?: boolean;
  /** Hide the per-stage explanations (tight layouts). */
  dense?: boolean;
  /** Fold the matrix and thresholds behind a disclosure. */
  collapsible?: boolean;
}) {
  const id = useId();
  const patch = (changes: Partial<SupervisionPolicy>) => onChange({ ...value, ...changes });

  const setOverride = (stage: Stage, override: Override) => {
    const overrides = { ...value.stage_overrides };
    if (override === "inherit") delete overrides[stage];
    else overrides[stage] = override;
    patch({ stage_overrides: overrides });
  };

  const overrideCount = Object.keys(value.stage_overrides).length;

  const advanced = (
    <div className="space-y-4">
        <div>
          <p className="mb-1.5 text-xs font-medium text-dim">
            Matrice par étape <span className="font-normal text-faint">— surcharge le mode par défaut, étape par étape</span>
          </p>
          <div className="overflow-x-auto rounded-md border border-line">
            <table className="w-full border-collapse text-xs">
              <thead className="bg-raised text-dim">
                <tr>
                  <th scope="col" className="px-2.5 py-1.5 text-left font-medium">
                    Étape
                  </th>
                  {(["inherit", "hitl", "hotl", "autonomous"] as Override[]).map((column) => (
                    <th key={column} scope="col" className="w-20 px-1 py-1.5 text-center font-medium">
                      {column === "inherit" ? "Hérité" : MODE_LABEL[column]}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {stages.map((stage) => {
                  const current: Override = value.stage_overrides[stage.id] ?? "inherit";
                  return (
                    <tr key={stage.id} className="border-t border-line">
                      <th scope="row" className="px-2.5 py-1.5 text-left font-normal">
                        <span className="text-ink">{STAGE_LABEL[stage.id]}</span>
                        {stage.key_checkpoint ? (
                          <span className="ml-1.5 text-[10px] text-warn" title="Étape clé : pause systématique en HITL">
                            étape clé
                          </span>
                        ) : null}
                        {dense ? null : <span className="block text-[11px] text-faint">{STAGE_HINT[stage.id]}</span>}
                      </th>
                      {(["inherit", "hitl", "hotl", "autonomous"] as Override[]).map((column) => (
                        <td key={column} className="px-1 py-1.5 text-center">
                          <input
                            type="radio"
                            name={`${id}-${stage.id}`}
                            checked={current === column}
                            onChange={() => setOverride(stage.id, column)}
                            aria-label={`${STAGE_LABEL[stage.id]} : ${column === "inherit" ? "hérité du mode par défaut" : MODE_LABEL[column]}`}
                            className="size-3.5 accent-accent"
                          />
                        </td>
                      ))}
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        </div>

        <div className="grid gap-3 sm:grid-cols-2">
          <div>
            <Label htmlFor={`${id}-confidence`}>
              Seuil de confiance <span className="font-mono text-ink">{formatPercent(value.confidence_threshold)}</span>
            </Label>
            <input
              id={`${id}-confidence`}
              type="range"
              min={0}
              max={1}
              step={0.05}
              value={value.confidence_threshold}
              onChange={(event) => patch({ confidence_threshold: Number(event.target.value) })}
              className="w-full accent-accent"
            />
            <p className="text-[11px] text-faint">En dessous, HOTL et HITL sollicitent un humain.</p>
          </div>
          <div>
            <Label htmlFor={`${id}-iterations`}>Itérations TDD avant divergence</Label>
            <Input
              id={`${id}-iterations`}
              type="number"
              min={1}
              max={20}
              value={value.max_tdd_iterations}
              onChange={(event) =>
                patch({ max_tdd_iterations: Math.max(1, Math.min(20, Math.round(Number(event.target.value)) || 1)) })
              }
            />
            <p className="mt-1 text-[11px] text-faint">Tentatives GREEN avant de lever une exception.</p>
          </div>
          <div>
            <Label htmlFor={`${id}-budget`}>Budget LLM (USD, optionnel)</Label>
            <Input
              id={`${id}-budget`}
              type="number"
              min={0}
              step={0.5}
              placeholder="illimité"
              value={value.budget_usd ?? ""}
              onChange={(event) => {
                const amount = Number(event.target.value);
                patch({ budget_usd: event.target.value === "" || !(amount > 0) ? null : amount });
              }}
            />
            <p className="mt-1 text-[11px] text-faint">Plafond de dépense ; son dépassement est une exception.</p>
          </div>
          <div>
            <Label htmlFor={`${id}-severity`}>Sévérité bloquante (sécurité)</Label>
            <Select
              id={`${id}-severity`}
              value={value.block_on_severity}
              onChange={(event) => patch({ block_on_severity: event.target.value as Severity })}
            >
              {SEVERITIES.map((severity) => (
                <option key={severity} value={severity}>
                  {SEVERITY_LABEL[severity]} et plus
                </option>
              ))}
            </Select>
            <p className="mt-1 text-[11px] text-faint">Un finding à ce niveau déclenche correction puis exception.</p>
          </div>
        </div>
    </div>
  );

  return (
    <fieldset disabled={disabled} className="space-y-4 disabled:opacity-60">
      <legend className="sr-only">Supervision</legend>

      <div role="radiogroup" aria-label="Mode de supervision par défaut" className="grid gap-2 sm:grid-cols-3">
        {MODES.map((mode) => {
          const selected = value.default_mode === mode.id;
          return (
            <button
              key={mode.id}
              type="button"
              role="radio"
              aria-checked={selected}
              onClick={() => patch({ default_mode: mode.id })}
              className={cn(
                "flex flex-col justify-start rounded-lg border p-3 text-left transition-colors",
                selected ? cn("bg-raised", mode.accent) : "border-line text-dim hover:border-line-strong hover:bg-raised/60",
              )}
            >
              <span className="flex w-full items-center gap-2 text-sm font-semibold [&_svg]:size-4">
                {mode.icon}
                {MODE_LABEL[mode.id]}
                <span className="ml-auto text-[11px] font-normal text-faint">{MODE_LONG[mode.id].tagline}</span>
              </span>
              <span className={cn("mt-1.5 block text-xs leading-relaxed", selected ? "text-ink" : "text-faint")}>
                {MODE_LONG[mode.id].detail}
              </span>
            </button>
          );
        })}
      </div>

      {collapsible ? (
        <details className="group rounded-md border border-line">
          <summary className="cursor-pointer list-none px-3 py-2 text-xs font-medium text-dim hover:text-ink">
            <span className="mr-1.5 inline-block transition-transform group-open:rotate-90">›</span>
            Matrice par étape et seuils d&apos;exception
            <span className="ml-2 font-normal text-faint">
              {overrideCount > 0 ? `${overrideCount} surcharge${overrideCount > 1 ? "s" : ""}` : "mode par défaut partout"} ·
              seuil {formatPercent(value.confidence_threshold)} · {value.max_tdd_iterations} itérations TDD
            </span>
          </summary>
          <div className="border-t border-line p-3">{advanced}</div>
        </details>
      ) : (
        advanced
      )}
    </fieldset>
  );
}
