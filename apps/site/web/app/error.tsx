"use client";

import { useEffect } from "react";
import { Link } from "@pylonsync/react";
import { AlertCircle, RefreshCw } from "lucide-react";
import { Button } from "@pylon-cloud/ui/components/ui/button";

// Root error boundary — catches anything thrown outside /dashboard
// (which has its own boundary). /login, /signup, /invite, /verify-email
// all funnel through here when their server-side pieces fail. Most
// commonly: the OAuth provider-list fetch timing out during a
// control-plane restart.
//
// Kept neutral on chrome — root pages don't share the dashboard's
// app shell, so this lives on a bare paper background rather than
// inside any sidebar/topbar.
export default function RootError({
	error,
	reset,
}: {
	error: Error & { digest?: string };
	reset: () => void;
}) {
	useEffect(() => {
		console.error("[root/error]", error.digest, error);
	}, [error]);

	const isUnreachable =
		error.name === "PylonUnreachableError" ||
		/timed out|unreachable|fetch failed/i.test(error.message);

	return (
		<div className="min-h-screen flex items-center justify-center bg-[var(--color-paper)] px-6">
			<div className="max-w-md w-full text-center space-y-4">
				<div className="inline-flex h-10 w-10 items-center justify-center rounded-full bg-[var(--color-paper-1)] text-[var(--color-status-warn)]">
					<AlertCircle className="size-5" />
				</div>
				<div className="space-y-1">
					<h1 className="text-[18px] font-semibold tracking-tight text-[var(--color-ink)]">
						{isUnreachable
							? "pylonsync.com is restarting"
							: "Something went wrong"}
					</h1>
					<p className="text-[13px] text-[var(--color-ink-3)] leading-relaxed">
						{isUnreachable
							? "The control plane didn't respond in time — usually means a deploy just landed. It comes back in a few seconds."
							: error.message || "An unexpected error occurred."}
					</p>
				</div>
				<div className="flex items-center justify-center gap-2 pt-2">
					<Button onClick={reset} variant="primary">
						<RefreshCw className="size-3.5" />
						Try again
					</Button>
					<Button asChild variant="default">
						<Link href="/">Home</Link>
					</Button>
				</div>
				{error.digest && (
					<div className="pt-3 text-[11px] font-mono text-[var(--color-ink-4)]">
						{error.digest}
					</div>
				)}
			</div>
		</div>
	);
}
