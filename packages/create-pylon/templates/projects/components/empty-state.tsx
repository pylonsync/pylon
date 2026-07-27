import React from "react";
import { cn } from "@/lib/utils";

/**
 * An empty state that says what this screen is for and offers the one action
 * that fills it. "No records found" tells a new user nothing and gives them
 * nowhere to go.
 */
export function EmptyState({
  icon,
  title,
  description,
  action,
  className,
}: {
  icon?: React.ReactNode;
  title: string;
  description?: string;
  action?: React.ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "flex flex-col items-center justify-center px-6 py-16 text-center",
        className,
      )}
    >
      {icon ? (
        <div className="mb-3 flex size-9 items-center justify-center rounded-lg bg-surface-2 text-muted-foreground [&_svg]:size-4">
          {icon}
        </div>
      ) : null}
      <p className="text-[13px] font-medium">{title}</p>
      {description ? (
        <p className="mt-1 max-w-sm text-[12px] leading-5 text-muted-foreground">
          {description}
        </p>
      ) : null}
      {action ? <div className="mt-4">{action}</div> : null}
    </div>
  );
}
