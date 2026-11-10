import type { HTMLAttributes, ReactNode } from "react";

import { cn } from "@/lib/utils";

/** Bordered dashboard surface. */
export function Panel({ className, ...props }: HTMLAttributes<HTMLDivElement>) {
  return <div className={cn("rounded-lg border border-line bg-panel", className)} {...props} />;
}

export function PanelHeader({
  title,
  icon,
  actions,
  className,
}: {
  title: ReactNode;
  icon?: ReactNode;
  actions?: ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("flex min-h-10 items-center justify-between gap-2 border-b border-line px-3 py-1.5", className)}>
      <h2 className="flex items-center gap-2 text-xs font-semibold tracking-wide text-dim uppercase [&_svg]:size-3.5">
        {icon}
        {title}
      </h2>
      {actions ? <div className="flex items-center gap-1.5">{actions}</div> : null}
    </div>
  );
}

/** Centered message for empty, loading and error states. */
export function EmptyState({
  icon,
  title,
  children,
  className,
}: {
  icon?: ReactNode;
  title: string;
  children?: ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("flex h-full min-h-28 flex-col items-center justify-center gap-1.5 p-6 text-center", className)}>
      {icon ? <div className="text-faint [&_svg]:size-6">{icon}</div> : null}
      <p className="text-sm font-medium text-dim">{title}</p>
      {children ? <div className="max-w-md text-xs leading-relaxed text-faint">{children}</div> : null}
    </div>
  );
}
