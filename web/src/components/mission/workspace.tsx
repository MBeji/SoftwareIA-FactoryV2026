"use client";

import { ChevronDown, ChevronRight, FileCode, Folder, FolderTree, RefreshCw } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { DiffFileView, DiffStat, DiffView, parseDiff } from "@/components/diff-view";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/panel";
import { Segmented } from "@/components/ui/segmented";
import { api } from "@/lib/api";
import { formatTime } from "@/lib/format";
import type { FileChangeEntry } from "@/lib/reducer";
import type { FileEntry, Meta } from "@/lib/types";
import { cn } from "@/lib/utils";

type Mode = "live" | "file" | "cumulative";

interface TreeDir {
  name: string;
  path: string;
  dirs: TreeDir[];
  files: FileEntry[];
}

function buildTree(files: FileEntry[]): TreeDir {
  const root: TreeDir = { name: "", path: "", dirs: [], files: [] };
  for (const file of files) {
    const parts = file.path.split("/");
    let current = root;
    for (const part of parts.slice(0, -1)) {
      let next = current.dirs.find((dir) => dir.name === part);
      if (!next) {
        next = { name: part, path: current.path ? `${current.path}/${part}` : part, dirs: [], files: [] };
        current.dirs.push(next);
      }
      current = next;
    }
    current.files.push(file);
  }
  return root;
}

type Stats = Map<string, { additions: number; deletions: number }>;

function DirView({
  dir,
  depth,
  stats,
  selected,
  onSelect,
}: {
  dir: TreeDir;
  depth: number;
  stats: Stats;
  selected: string | null;
  onSelect: (path: string) => void;
}) {
  const [open, setOpen] = useState(true);
  return (
    <li>
      {dir.name ? (
        <button
          type="button"
          onClick={() => setOpen((value) => !value)}
          aria-expanded={open}
          className="flex w-full items-center gap-1 rounded px-1 py-0.5 text-left text-xs text-dim hover:bg-hover"
          style={{ paddingLeft: depth * 12 + 4 }}
        >
          {open ? <ChevronDown className="size-3 shrink-0" /> : <ChevronRight className="size-3 shrink-0" />}
          <Folder className="size-3.5 shrink-0 text-faint" />
          <span className="truncate">{dir.name}</span>
        </button>
      ) : null}
      {open ? (
        <ul>
          {dir.dirs.map((child) => (
            <DirView key={child.path} dir={child} depth={dir.name ? depth + 1 : depth} stats={stats} selected={selected} onSelect={onSelect} />
          ))}
          {dir.files.map((file) => {
            const name = file.path.split("/").pop() ?? file.path;
            const stat = stats.get(file.path);
            return (
              <li key={file.path}>
                <button
                  type="button"
                  onClick={() => onSelect(file.path)}
                  aria-current={selected === file.path ? "true" : undefined}
                  className={cn(
                    "flex w-full items-center gap-1.5 rounded px-1 py-0.5 text-left text-xs hover:bg-hover",
                    selected === file.path ? "bg-accent/12 text-accent" : "text-ink",
                  )}
                  style={{ paddingLeft: (dir.name ? depth + 1 : depth) * 12 + 18 }}
                >
                  <FileCode className="size-3.5 shrink-0 text-faint" />
                  <span className="min-w-0 flex-1 truncate font-mono text-[11px]">{name}</span>
                  {stat ? <DiffStat additions={stat.additions} deletions={stat.deletions} /> : null}
                </button>
              </li>
            );
          })}
        </ul>
      ) : null}
    </li>
  );
}

function FileViewer({ runId, path, revision }: { runId: string; path: string; revision: number }) {
  const [loaded, setLoaded] = useState<{ path: string; content: string } | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let disposed = false;
    api.file(runId, path).then(
      (file) => {
        if (disposed) return;
        setLoaded(file);
        setError(null);
      },
      (reason: unknown) => {
        if (!disposed) setError(reason instanceof Error ? reason.message : "Erreur inconnue");
      },
    );
    return () => {
      disposed = true;
    };
  }, [runId, path, revision]);

  if (error) return <p className="p-3 text-xs text-bad">Lecture impossible : {error}</p>;
  if (!loaded || loaded.path !== path) return <p className="p-3 text-xs text-faint">Chargement de {path}…</p>;
  const lines = loaded.content.replace(/\n$/, "").split("\n");
  return (
    <div className="overflow-hidden rounded-md border border-line bg-canvas">
      <div className="border-b border-line bg-raised px-2.5 py-1.5 font-mono text-xs text-ink">{path}</div>
      <div className="overflow-x-auto">
        <table className="border-collapse font-mono text-xs leading-5">
          <tbody>
            {lines.map((line, index) => (
              <tr key={index}>
                <td className="w-10 border-r border-line px-1.5 text-right text-faint select-none">{index + 1}</td>
                <td className="px-3 whitespace-pre text-ink">{line || " "}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

/** File explorer, file viewer and git-style diffs (live, per event, and cumulative). */
export function WorkspacePane({
  runId,
  changes,
  revision,
  meta,
}: {
  runId: string;
  changes: FileChangeEntry[];
  /** Bumps whenever something may have touched the workspace. */
  revision: number;
  meta: Meta | null;
}) {
  const [files, setFiles] = useState<FileEntry[] | null>(null);
  const [treeError, setTreeError] = useState<string | null>(null);
  const [mode, setMode] = useState<Mode>("live");
  const [selected, setSelected] = useState<string | null>(null);
  const [cumulative, setCumulative] = useState<string | null>(null);
  const [cumulativeError, setCumulativeError] = useState<string | null>(null);
  const [manual, setManual] = useState(0);

  useEffect(() => {
    let disposed = false;
    const timer = setTimeout(() => {
      api.tree(runId).then(
        (tree) => {
          if (disposed) return;
          setFiles(tree);
          setTreeError(null);
        },
        (reason: unknown) => {
          if (!disposed) setTreeError(reason instanceof Error ? reason.message : "Erreur inconnue");
        },
      );
    }, 250);
    return () => {
      disposed = true;
      clearTimeout(timer);
    };
  }, [runId, revision]);

  useEffect(() => {
    if (mode !== "cumulative") return;
    let disposed = false;
    const timer = setTimeout(() => {
      api.diff(runId).then(
        (diff) => {
          if (disposed) return;
          setCumulative(diff);
          setCumulativeError(null);
        },
        (reason: unknown) => {
          if (!disposed) setCumulativeError(reason instanceof Error ? reason.message : "Erreur inconnue");
        },
      );
    }, 400);
    return () => {
      disposed = true;
      clearTimeout(timer);
    };
  }, [runId, mode, revision, manual]);

  const tree = useMemo(() => buildTree(files ?? []), [files]);
  const stats = useMemo<Stats>(() => {
    const map: Stats = new Map();
    for (const change of changes) {
      const current = map.get(change.path) ?? { additions: 0, deletions: 0 };
      map.set(change.path, { additions: current.additions + change.additions, deletions: current.deletions + change.deletions });
    }
    return map;
  }, [changes]);
  const roleName = (id: string | null): string => (id ? (meta?.roles.find((role) => role.id === id)?.name ?? id) : "orchestrateur");
  const fileChanges = selected ? changes.filter((change) => change.path === selected) : [];
  const live = [...changes].reverse().slice(0, 40);

  return (
    <div className="flex h-full min-h-0">
      <div className="flex w-52 shrink-0 flex-col border-r border-line lg:w-60">
        <p className="flex items-center justify-between border-b border-line px-2.5 py-1.5 text-[11px] text-faint">
          <span className="flex items-center gap-1.5">
            <FolderTree className="size-3.5" /> Fichiers
          </span>
          <span className="font-mono">{files?.length ?? "…"}</span>
        </p>
        <div className="min-h-0 flex-1 overflow-y-auto p-1">
          {treeError ? (
            <p className="p-2 text-xs text-faint">Workspace indisponible : {treeError}</p>
          ) : !files ? (
            <p className="p-2 text-xs text-faint">Chargement…</p>
          ) : files.length === 0 ? (
            <p className="p-2 text-xs text-faint">Workspace vide.</p>
          ) : (
            <ul>
              <DirView
                dir={tree}
                depth={0}
                stats={stats}
                selected={mode === "file" ? selected : null}
                onSelect={(path) => {
                  setSelected(path);
                  setMode("file");
                }}
              />
            </ul>
          )}
        </div>
      </div>

      <div className="flex min-w-0 flex-1 flex-col">
        <div className="flex shrink-0 flex-wrap items-center justify-between gap-2 border-b border-line px-2.5 py-1.5">
          <Segmented<Mode>
            label="Vue du workspace"
            value={mode}
            onChange={setMode}
            options={[
              { value: "live", label: `Diffs en direct (${changes.length})`, title: "Chaque écriture d'un agent, au fil de l'eau" },
              { value: "file", label: "Fichier", title: "Contenu et historique du fichier sélectionné" },
              { value: "cumulative", label: "Diff cumulé vs main", title: "git diff de la branche du run contre sa base" },
            ]}
          />
          {mode === "cumulative" ? (
            <Button size="sm" variant="ghost" onClick={() => setManual((value) => value + 1)}>
              <RefreshCw /> Actualiser
            </Button>
          ) : null}
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto p-2.5">
          {mode === "live" ? (
            live.length === 0 ? (
              <EmptyState icon={<FileCode />} title="Aucune écriture pour l'instant">
                Chaque fichier créé ou modifié par un agent apparaît ici avec son diff, au moment où il est écrit.
              </EmptyState>
            ) : (
              <ol className="space-y-3">
                {live.map((change) => {
                  const parsed = parseDiff(change.diff)[0];
                  return (
                    <li key={change.seq}>
                      <p className="mb-1 flex flex-wrap items-center gap-x-2 text-[11px] text-faint">
                        <span className="font-mono">{formatTime(change.ts)}</span>
                        <span className="text-dim">{roleName(change.role)}</span>
                        {change.taskId ? <span className="font-mono">{change.taskId}</span> : null}
                        <span>{change.created ? "crée" : "modifie"}</span>
                      </p>
                      {parsed ? (
                        <DiffFileView file={{ ...parsed, path: change.path, created: change.created }} />
                      ) : (
                        <p className="rounded-md border border-line px-2.5 py-1.5 font-mono text-xs text-dim">{change.path} (fichier vide)</p>
                      )}
                    </li>
                  );
                })}
              </ol>
            )
          ) : mode === "file" ? (
            selected ? (
              <div className="space-y-3">
                <FileViewer runId={runId} path={selected} revision={revision} />
                {fileChanges.length > 0 ? (
                  <div>
                    <p className="mb-1.5 text-[11px] font-semibold tracking-wide text-dim uppercase">
                      Historique des écritures ({fileChanges.length})
                    </p>
                    <ol className="space-y-2">
                      {[...fileChanges].reverse().map((change) => {
                        const parsed = parseDiff(change.diff)[0];
                        return (
                          <li key={change.seq}>
                            <p className="mb-1 text-[11px] text-faint">
                              <span className="font-mono">{formatTime(change.ts)}</span> · {roleName(change.role)}
                            </p>
                            {parsed ? <DiffFileView file={{ ...parsed, path: change.path, created: change.created }} /> : null}
                          </li>
                        );
                      })}
                    </ol>
                  </div>
                ) : null}
              </div>
            ) : (
              <EmptyState icon={<FileCode />} title="Sélectionnez un fichier dans l'explorateur" />
            )
          ) : cumulativeError ? (
            <EmptyState title="Diff indisponible">{cumulativeError}</EmptyState>
          ) : cumulative === null ? (
            <p className="text-xs text-faint">Calcul du diff…</p>
          ) : (
            <DiffView diff={cumulative} emptyLabel="Aucune différence avec la branche de base pour l'instant." />
          )}
        </div>
      </div>
    </div>
  );
}
