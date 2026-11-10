import { FileDiff } from "lucide-react";

import { cn } from "@/lib/utils";

export interface DiffLine {
  type: "add" | "del" | "ctx" | "meta";
  oldNo: number | null;
  newNo: number | null;
  text: string;
}

export interface DiffHunk {
  header: string;
  lines: DiffLine[];
}

export interface DiffFile {
  path: string;
  created: boolean;
  deleted: boolean;
  additions: number;
  deletions: number;
  hunks: DiffHunk[];
}

const stripPrefix = (path: string): string => path.replace(/^[ab]\//, "").trim();

/** Parses unified diffs (plain `---/+++` pairs or full `git diff` output, multi-file). */
export function parseDiff(text: string): DiffFile[] {
  const files: DiffFile[] = [];
  let file: DiffFile | null = null;
  let hunk: DiffHunk | null = null;
  let oldNo = 0;
  let newNo = 0;

  const open = (path: string): DiffFile => {
    const created: DiffFile = { path, created: false, deleted: false, additions: 0, deletions: 0, hunks: [] };
    files.push(created);
    hunk = null;
    return created;
  };

  for (const line of text.split("\n")) {
    if (line.startsWith("diff --git ")) {
      const match = / b\/(.+)$/.exec(line);
      file = open(match ? match[1] : line.slice(11));
      continue;
    }
    if (line.startsWith("--- ")) {
      if (!file || file.hunks.length > 0) file = open("");
      const source = line.slice(4).trim();
      if (source === "/dev/null") file.created = true;
      else if (!file.path) file.path = stripPrefix(source);
      continue;
    }
    if (line.startsWith("+++ ")) {
      if (!file) file = open("");
      const target = line.slice(4).trim();
      if (target === "/dev/null") file.deleted = true;
      else file.path = stripPrefix(target);
      continue;
    }
    if (line.startsWith("@@")) {
      if (!file) file = open("");
      const match = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(line);
      oldNo = match ? Number(match[1]) : 1;
      newNo = match ? Number(match[2]) : 1;
      hunk = { header: line, lines: [] };
      file.hunks.push(hunk);
      continue;
    }
    if (!file || !hunk) continue; // file-level metadata (index, mode…) before the first hunk
    const active: DiffHunk = hunk;
    if (line.startsWith("+")) {
      active.lines.push({ type: "add", oldNo: null, newNo, text: line.slice(1) });
      newNo += 1;
      file.additions += 1;
    } else if (line.startsWith("-")) {
      active.lines.push({ type: "del", oldNo, newNo: null, text: line.slice(1) });
      oldNo += 1;
      file.deletions += 1;
    } else if (line.startsWith("\\")) {
      active.lines.push({ type: "meta", oldNo: null, newNo: null, text: line.slice(2) });
    } else if (line.startsWith(" ") || line === "") {
      // A bare empty line at the very end of the diff is just the trailing newline.
      if (line === "" && active.lines.length === 0) continue;
      active.lines.push({ type: "ctx", oldNo, newNo, text: line.slice(1) });
      oldNo += 1;
      newNo += 1;
    }
  }
  // Drop the phantom context line produced by a trailing newline.
  for (const parsed of files) {
    const last = parsed.hunks[parsed.hunks.length - 1];
    const tail = last?.lines[last.lines.length - 1];
    if (tail && tail.type === "ctx" && tail.text === "" && text.endsWith("\n")) last.lines.pop();
  }
  return files.filter((parsed) => parsed.hunks.length > 0 || parsed.path);
}

const LINE_STYLE: Record<DiffLine["type"], string> = {
  add: "bg-ok/10 text-ink",
  del: "bg-bad/10 text-ink",
  ctx: "text-dim",
  meta: "text-faint italic",
};

const SIGN: Record<DiffLine["type"], string> = { add: "+", del: "−", ctx: " ", meta: " " };

export function DiffStat({ additions, deletions }: { additions: number; deletions: number }) {
  return (
    <span className="font-mono text-[11px] whitespace-nowrap">
      <span className="text-ok">+{additions}</span> <span className="text-bad">−{deletions}</span>
    </span>
  );
}

/** Git-style unified diff with old/new line numbers. */
export function DiffFileView({ file, className }: { file: DiffFile; className?: string }) {
  return (
    <div className={cn("overflow-hidden rounded-md border border-line bg-canvas", className)}>
      <div className="flex items-center justify-between gap-2 border-b border-line bg-raised px-2.5 py-1.5">
        <span className="flex min-w-0 items-center gap-1.5 font-mono text-xs text-ink">
          <FileDiff className="size-3.5 shrink-0 text-faint" />
          <span className="truncate">{file.path || "(fichier)"}</span>
          {file.created ? <span className="text-ok">nouveau</span> : null}
          {file.deleted ? <span className="text-bad">supprimé</span> : null}
        </span>
        <DiffStat additions={file.additions} deletions={file.deletions} />
      </div>
      <div className="overflow-x-auto">
        <table className="w-full border-collapse font-mono text-xs leading-5">
          <tbody>
            {file.hunks.map((hunk, hunkIndex) => (
              <HunkRows key={hunkIndex} hunk={hunk} />
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function HunkRows({ hunk }: { hunk: DiffHunk }) {
  return (
    <>
      <tr className="bg-info/8 text-info">
        <td colSpan={4} className="px-2.5 py-0.5 select-none">
          {hunk.header}
        </td>
      </tr>
      {hunk.lines.map((line, index) => (
        <tr key={index} className={LINE_STYLE[line.type]}>
          <td className="w-10 border-r border-line px-1.5 text-right text-faint select-none">{line.oldNo ?? ""}</td>
          <td className="w-10 border-r border-line px-1.5 text-right text-faint select-none">{line.newNo ?? ""}</td>
          <td
            className={cn(
              "w-5 px-1 text-center select-none",
              line.type === "add" && "text-ok",
              line.type === "del" && "text-bad",
            )}
          >
            {SIGN[line.type]}
          </td>
          <td className="pr-3 whitespace-pre">{line.text || " "}</td>
        </tr>
      ))}
    </>
  );
}

export function DiffView({ diff, emptyLabel = "Aucune différence." }: { diff: string; emptyLabel?: string }) {
  const files = parseDiff(diff);
  if (files.length === 0) return <p className="p-4 text-xs text-faint">{emptyLabel}</p>;
  return (
    <div className="space-y-3">
      {files.map((file, index) => (
        <DiffFileView key={`${file.path}-${index}`} file={file} />
      ))}
    </div>
  );
}
