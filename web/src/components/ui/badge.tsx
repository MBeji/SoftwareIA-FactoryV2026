import { cva, type VariantProps } from "class-variance-authority";
import type { HTMLAttributes } from "react";

import { cn } from "@/lib/utils";

const badgeVariants = cva(
  "inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[11px] font-medium leading-4 whitespace-nowrap [&_svg]:size-3",
  {
    variants: {
      tone: {
        neutral: "bg-raised text-dim border border-line",
        accent: "bg-accent/12 text-accent border border-accent/30",
        ok: "bg-ok/12 text-ok border border-ok/30",
        warn: "bg-warn/12 text-warn border border-warn/30",
        bad: "bg-bad/12 text-bad border border-bad/30",
        violet: "bg-violet/12 text-violet border border-violet/30",
        info: "bg-info/12 text-info border border-info/30",
      },
    },
    defaultVariants: { tone: "neutral" },
  },
);

export type BadgeTone = NonNullable<VariantProps<typeof badgeVariants>["tone"]>;

export function Badge({ className, tone, ...props }: HTMLAttributes<HTMLSpanElement> & { tone?: BadgeTone }) {
  return <span className={cn(badgeVariants({ tone }), className)} {...props} />;
}
