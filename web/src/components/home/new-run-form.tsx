"use client";

import { FileText, FlaskConical, Play, ScanSearch, TriangleAlert, Upload, X } from "lucide-react";
import { useRouter } from "next/navigation";
import { useRef, useState, type FormEvent } from "react";

import { ClassificationSummary, PlanView } from "@/components/plan-view";
import { SupervisionSelector } from "@/components/supervision-selector";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input, Label, Select, Textarea } from "@/components/ui/field";
import { Panel, PanelHeader } from "@/components/ui/panel";
import { Segmented } from "@/components/ui/segmented";
import { api } from "@/lib/api";
import { useDemoMode } from "@/lib/demo";
import { PRESETS } from "@/lib/presets";
import type { Meta, PlanPreview, SupervisionPolicy } from "@/lib/types";

type Source = "text" | "file";
type LevelChoice = "auto" | "1" | "2" | "3";

const DIRECTIVES = [
  { tag: "[sim:diverge]", label: "boucle TDD divergente" },
  { tag: "[sim:low-confidence]", label: "architecte peu confiant" },
];

const message = (reason: unknown): string => (reason instanceof Error ? reason.message : "Erreur inconnue");

export function NewRunForm({ meta }: { meta: Meta }) {
  const router = useRouter();
  const fileInput = useRef<HTMLInputElement>(null);
  const [source, setSource] = useState<Source>("text");
  const [text, setText] = useState("");
  const [file, setFile] = useState<File | null>(null);
  const [title, setTitle] = useState("");
  const [level, setLevel] = useState<LevelChoice>("auto");
  const [policy, setPolicy] = useState<SupervisionPolicy>(meta.supervision);
  const [preview, setPreview] = useState<PlanPreview | null>(null);
  const [busy, setBusy] = useState<"preview" | "launch" | null>(null);
  const [error, setError] = useState<string | null>(null);

  const demo = useDemoMode();
  const simulated = meta.llm_backend === "simulated";
  const levelValue = level === "auto" ? null : Number(level);
  const isPdf = source === "file" && Boolean(file?.name.toLowerCase().endsWith(".pdf"));
  const ready = source === "text" ? text.trim().length > 0 : file !== null;

  const invalidate = () => {
    setPreview(null);
    setError(null);
  };

  const analyse = async () => {
    setBusy("preview");
    setError(null);
    try {
      const input = source === "text" ? text : await file!.text();
      setPreview(await api.previewPlan(input, levelValue));
    } catch (reason) {
      setPreview(null);
      setError(message(reason));
    } finally {
      setBusy(null);
    }
  };

  const launch = async (event: FormEvent) => {
    event.preventDefault();
    if (!ready) return;
    setBusy("launch");
    setError(null);
    try {
      const options = { title: title.trim(), level: levelValue, supervision: policy };
      const run =
        source === "text" ? await api.createRun({ input: text, ...options }) : await api.uploadRun(file!, options);
      router.push(`/runs/${run.id}`);
    } catch (reason) {
      setError(message(reason));
      setBusy(null);
    }
  };

  return (
    <Panel>
      <PanelHeader icon={<Play />} title="Nouveau run" />
      <form onSubmit={launch} className="space-y-5 p-4">
        <section>
          <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
            <p className="text-xs font-medium text-dim">1 · La demande — de la micro-tâche au cahier des charges</p>
            <Segmented<Source>
              label="Source de la demande"
              value={source}
              onChange={(next) => {
                setSource(next);
                invalidate();
              }}
              options={[
                { value: "text", label: "Texte" },
                { value: "file", label: "Document" },
              ]}
            />
          </div>

          {source === "text" ? (
            <>
              <div className="mb-2 flex flex-wrap items-center gap-1.5">
                <span className="text-[11px] text-faint">Exemples :</span>
                {PRESETS.map((preset) => (
                  <Button
                    key={preset.id}
                    size="sm"
                    variant="outline"
                    title={preset.hint}
                    onClick={() => {
                      setText(preset.input);
                      invalidate();
                    }}
                  >
                    <Badge tone="accent">N{preset.level}</Badge> {preset.label}
                  </Button>
                ))}
              </div>
              <Label htmlFor="run-input" className="sr-only">
                Demande
              </Label>
              <Textarea
                id="run-input"
                value={text}
                onChange={(event) => {
                  setText(event.target.value);
                  invalidate();
                }}
                rows={text.split("\n").length > 6 ? 14 : 5}
                placeholder="Décrivez une user story, une epic, ou collez un cahier des charges en Markdown…"
                className="font-mono text-[13px]"
              />
              {simulated && !demo ? (
                <p className="mt-1.5 flex flex-wrap items-center gap-1.5 text-[11px] text-faint">
                  <FlaskConical className="size-3.5" />
                  Backend simulé — directives de démonstration :
                  {DIRECTIVES.map((directive) => (
                    <button
                      key={directive.tag}
                      type="button"
                      title={`Ajouter ${directive.tag} à la demande`}
                      onClick={() => {
                        if (!text.includes(directive.tag)) setText((value) => `${value.trimEnd()} ${directive.tag}`.trim());
                      }}
                      className="rounded border border-line px-1.5 py-0.5 font-mono text-dim hover:border-line-strong hover:text-ink"
                    >
                      {directive.tag} <span className="font-sans text-faint">({directive.label})</span>
                    </button>
                  ))}
                </p>
              ) : null}
            </>
          ) : (
            <div className="rounded-md border border-dashed border-line-strong bg-canvas p-4">
              <input
                ref={fileInput}
                id="run-file"
                type="file"
                accept=".md,.markdown,.txt,.pdf"
                className="sr-only"
                onChange={(event) => {
                  setFile(event.target.files?.[0] ?? null);
                  invalidate();
                }}
              />
              {file ? (
                <div className="flex items-center justify-between gap-3">
                  <span className="flex min-w-0 items-center gap-2 text-sm text-ink">
                    <FileText className="size-4 shrink-0 text-accent" />
                    <span className="truncate">{file.name}</span>
                    <span className="shrink-0 font-mono text-[11px] text-faint">{Math.ceil(file.size / 1024)} ko</span>
                  </span>
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => {
                      setFile(null);
                      invalidate();
                      if (fileInput.current) fileInput.current.value = "";
                    }}
                  >
                    <X /> Retirer
                  </Button>
                </div>
              ) : (
                <label htmlFor="run-file" className="flex cursor-pointer flex-col items-center gap-1.5 py-3 text-center">
                  <Upload className="size-5 text-faint" />
                  <span className="text-sm text-ink">Choisir un cahier des charges</span>
                  <span className="text-xs text-faint">Markdown, texte ou PDF — l&apos;extraction se fait côté serveur</span>
                </label>
              )}
            </div>
          )}
        </section>

        <section className="grid gap-3 sm:grid-cols-[1fr_14rem]">
          <div>
            <Label htmlFor="run-title">Titre du run (optionnel)</Label>
            <Input
              id="run-title"
              value={title}
              onChange={(event) => setTitle(event.target.value)}
              placeholder="Par défaut : première ligne de la demande"
              maxLength={120}
            />
          </div>
          <div>
            <Label htmlFor="run-level">Granularité</Label>
            <Select
              id="run-level"
              value={level}
              onChange={(event) => {
                setLevel(event.target.value as LevelChoice);
                invalidate();
              }}
            >
              <option value="auto">Automatique (classifieur)</option>
              <option value="1">Niveau 1 — micro-tâche</option>
              <option value="2">Niveau 2 — epic</option>
              <option value="3">Niveau 3 — cahier des charges</option>
            </Select>
          </div>
        </section>

        <section>
          <p className="mb-2 text-xs font-medium text-dim">2 · La supervision — qui décide, et quand</p>
          <SupervisionSelector value={policy} onChange={setPolicy} stages={meta.stages} collapsible />
        </section>

        {error ? (
          <p role="alert" className="flex items-start gap-2 rounded-md border border-bad/40 bg-bad/10 px-3 py-2 text-sm text-bad">
            <TriangleAlert className="mt-0.5 size-4 shrink-0" /> {error}
          </p>
        ) : null}

        <div className="flex flex-wrap items-center justify-end gap-2 border-t border-line pt-4">
          {isPdf ? (
            <span className="mr-auto text-xs text-faint">
              Aperçu indisponible pour un PDF : le texte est extrait par le serveur au lancement.
            </span>
          ) : null}
          <Button variant="secondary" disabled={!ready || isPdf || busy !== null} onClick={() => void analyse()}>
            <ScanSearch /> {busy === "preview" ? "Analyse…" : "Analyser"}
          </Button>
          {demo ? (
            <span className="mr-auto max-w-md text-xs text-faint">
              Mode démonstration : le lancement demande un backend. « Analyser » fonctionne sur les trois exemples ;
              les runs enregistrés sont dans la liste.
            </span>
          ) : null}
          <Button type="submit" variant="primary" size="lg" disabled={demo || !ready || busy !== null}>
            <Play /> {busy === "launch" ? "Lancement…" : "Lancer le run"}
          </Button>
        </div>

        {preview ? (
          <section aria-label="Aperçu du plan" className="space-y-3 border-t border-line pt-4">
            <div className="flex items-center justify-between gap-2">
              <h3 className="text-sm font-semibold text-ink">Aperçu de l&apos;ingestion</h3>
              <span className="text-[11px] text-faint">
                Décomposition déterministe, sans appel LLM. Le Product Owner peut l&apos;affiner au lancement.
              </span>
            </div>
            <ClassificationSummary classification={preview.classification} />
            <PlanView plan={preview.plan} />
          </section>
        ) : null}
      </form>
    </Panel>
  );
}
