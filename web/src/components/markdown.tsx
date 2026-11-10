import { Fragment, type ReactNode } from "react";

import { cn } from "@/lib/utils";

// Minimal, dependency-free Markdown renderer for agent-written documents (architecture,
// merge request body). It builds React nodes only — no HTML injection.

function inline(text: string): ReactNode[] {
  const parts = text.split(/(`[^`]+`|\*\*[^*]+\*\*)/g);
  return parts.map((part, index) => {
    if (part.startsWith("`") && part.endsWith("`") && part.length > 2) {
      return (
        <code key={index} className="rounded bg-raised px-1 py-0.5 font-mono text-[0.85em] text-accent">
          {part.slice(1, -1)}
        </code>
      );
    }
    if (part.startsWith("**") && part.endsWith("**") && part.length > 4) {
      return (
        <strong key={index} className="font-semibold text-ink">
          {part.slice(2, -2)}
        </strong>
      );
    }
    return <Fragment key={index}>{part}</Fragment>;
  });
}

const cells = (row: string): string[] =>
  row
    .trim()
    .replace(/^\||\|$/g, "")
    .split("|")
    .map((cell) => cell.trim());

const isDivider = (row: string): boolean => /^\s*\|?\s*:?-{2,}/.test(row) && /^[\s|:-]+$/.test(row);

export function Markdown({ source, className }: { source: string; className?: string }) {
  const lines = source.replace(/\r\n/g, "\n").split("\n");
  const blocks: ReactNode[] = [];
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];
    const key = blocks.length;

    if (line.trim() === "") {
      i += 1;
      continue;
    }
    if (line.trimStart().startsWith("```")) {
      const code: string[] = [];
      i += 1;
      while (i < lines.length && !lines[i].trimStart().startsWith("```")) {
        code.push(lines[i]);
        i += 1;
      }
      i += 1;
      blocks.push(
        <pre key={key} className="overflow-x-auto rounded-md border border-line bg-canvas p-3 font-mono text-xs leading-relaxed text-ink">
          {code.join("\n")}
        </pre>,
      );
      continue;
    }
    const heading = /^(#{1,6})\s+(.*)$/.exec(line);
    if (heading) {
      const level = heading[1].length;
      const style =
        level === 1
          ? "text-base font-semibold text-ink"
          : level === 2
            ? "mt-1 text-sm font-semibold text-ink"
            : "text-xs font-semibold tracking-wide text-dim uppercase";
      blocks.push(
        <p key={key} role="heading" aria-level={Math.min(level + 2, 6)} className={style}>
          {inline(heading[2])}
        </p>,
      );
      i += 1;
      continue;
    }
    if (line.trimStart().startsWith("|")) {
      const rows: string[] = [];
      while (i < lines.length && lines[i].trimStart().startsWith("|")) {
        rows.push(lines[i]);
        i += 1;
      }
      const hasHeader = rows.length > 1 && isDivider(rows[1]);
      const body = rows.filter((row, index) => !(isDivider(row) && index > 0)).slice(hasHeader ? 1 : 0);
      blocks.push(
        <div key={key} className="overflow-x-auto rounded-md border border-line">
          <table className="w-full border-collapse text-left text-xs">
            {hasHeader ? (
              <thead className="bg-raised text-dim">
                <tr>
                  {cells(rows[0]).map((cell, index) => (
                    <th key={index} className="px-2.5 py-1.5 font-medium">
                      {inline(cell)}
                    </th>
                  ))}
                </tr>
              </thead>
            ) : null}
            <tbody>
              {body.map((row, rowIndex) => (
                <tr key={rowIndex} className="border-t border-line">
                  {cells(row).map((cell, index) => (
                    <td key={index} className="px-2.5 py-1.5 align-top text-ink">
                      {inline(cell)}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>,
      );
      continue;
    }
    if (/^\s*(?:[-*+]|\d+[.)])\s+/.test(line)) {
      const ordered = /^\s*\d+[.)]\s+/.test(line);
      const items: { depth: number; text: string }[] = [];
      while (i < lines.length && /^\s*(?:[-*+]|\d+[.)])\s+/.test(lines[i])) {
        const match = /^(\s*)(?:[-*+]|\d+[.)])\s+(.*)$/.exec(lines[i]);
        if (match) items.push({ depth: Math.min(Math.floor(match[1].length / 2), 3), text: match[2] });
        i += 1;
      }
      const List = ordered ? "ol" : "ul";
      blocks.push(
        <List key={key} className={cn("space-y-1 pl-5 text-sm text-ink", ordered ? "list-decimal" : "list-disc")}>
          {items.map((item, index) => (
            <li key={index} style={{ marginLeft: item.depth * 16 }} className="leading-relaxed marker:text-faint">
              {inline(item.text)}
            </li>
          ))}
        </List>,
      );
      continue;
    }
    if (line.trimStart().startsWith(">")) {
      const quote: string[] = [];
      while (i < lines.length && lines[i].trimStart().startsWith(">")) {
        quote.push(lines[i].trimStart().replace(/^>\s?/, ""));
        i += 1;
      }
      blocks.push(
        <blockquote key={key} className="border-l-2 border-line-strong pl-3 text-sm text-dim italic">
          {inline(quote.join(" "))}
        </blockquote>,
      );
      continue;
    }
    const paragraph: string[] = [];
    while (
      i < lines.length &&
      lines[i].trim() !== "" &&
      !/^(#{1,6}\s|\s*(?:[-*+]|\d+[.)])\s+|\s*\||\s*>|\s*```)/.test(lines[i])
    ) {
      paragraph.push(lines[i].trim());
      i += 1;
    }
    blocks.push(
      <p key={key} className="text-sm leading-relaxed text-ink">
        {inline(paragraph.join(" "))}
      </p>,
    );
  }

  return <div className={cn("space-y-3", className)}>{blocks}</div>;
}
