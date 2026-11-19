import * as React from "react";
import { AlertCircle, Loader2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";

/** What a data view shows when it has nothing to show. */
export function EmptyState({
	title,
	description,
	action,
	className,
}: {
	title: React.ReactNode;
	description?: React.ReactNode;
	action?: React.ReactNode;
	className?: string;
}) {
	return (
		<div
			className={cn(
				"flex flex-col items-center justify-center gap-1 px-4 py-14 text-center",
				className,
			)}
		>
			<p className="text-[13px] font-medium">{title}</p>
			{description && (
				<p className="max-w-sm text-xs text-muted-foreground">{description}</p>
			)}
			{action && <div className="mt-2">{action}</div>}
		</div>
	);
}

/**
 * What a data view shows when its request fails: what failed, the message
 * from the server, and a button that tries again.
 */
export function ErrorState({
	title,
	message,
	onRetry,
	className,
}: {
	title: React.ReactNode;
	message?: React.ReactNode;
	onRetry?: () => unknown;
	className?: string;
}) {
	return (
		<div
			role="alert"
			className={cn(
				"flex flex-col items-center justify-center gap-1 px-4 py-14 text-center",
				className,
			)}
		>
			<AlertCircle className="mb-1 size-4 text-status-red-fg" />
			<p className="text-[13px] font-medium">{title}</p>
			{message && (
				<p className="max-w-md font-mono text-xs break-words text-muted-foreground">
					{message}
				</p>
			)}
			{onRetry && (
				<Button variant="outline" className="mt-2" onClick={onRetry}>
					Try again
				</Button>
			)}
		</div>
	);
}

/** A one-line error above content that still shows the last good data. */
export function ErrorBanner({
	children,
	className,
}: {
	children: React.ReactNode;
	className?: string;
}) {
	return (
		<div
			role="alert"
			className={cn(
				"flex items-start gap-2 border-b bg-status-red-bg px-3 py-2 text-xs text-status-red-fg",
				className,
			)}
		>
			<AlertCircle className="mt-px size-3.5 shrink-0" />
			<span className="min-w-0 break-words">{children}</span>
		</div>
	);
}

/** Placeholder rows for a table or a list that is loading. */
export function LoadingRows({
	rows = 8,
	className,
}: {
	rows?: number;
	className?: string;
}) {
	return (
		<div role="status" aria-label="Loading" className={className}>
			{Array.from({ length: rows }, (_, i) => (
				<div key={i} className="flex h-8 items-center gap-6 border-b px-3">
					<Skeleton className="w-16" />
					<Skeleton className="w-40" />
					<Skeleton className="w-24" />
					<Skeleton className="hidden w-32 sm:block" />
				</div>
			))}
		</div>
	);
}

/** A spinner for a button or a small inline wait. */
export function Spinner({ className }: { className?: string }) {
	return (
		<Loader2
			aria-hidden="true"
			className={cn("size-3.5 animate-spin motion-reduce:animate-none", className)}
		/>
	);
}
