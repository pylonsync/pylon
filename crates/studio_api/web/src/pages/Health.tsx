import { useCallback, useEffect, useState } from "react";
import { Loader2, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
	Card,
	CardContent,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import { ApiError, api } from "@/lib/pylon";
import { LockedPage } from "./Locked";
import { useAuth } from "@/auth/AuthContext";

type Health = {
	status?: string;
	version?: string;
	uptime_secs?: number;
	uptime?: number;
};

type Metrics = {
	total_requests?: number;
	error_count?: number;
	avg_response_ms?: number;
	active_connections?: number;
	by_method?: Record<string, number>;
	[k: string]: unknown;
};

function formatUptime(s: number) {
	const h = Math.floor(s / 3600);
	const m = Math.floor((s % 3600) / 60);
	const sec = Math.floor(s % 60);
	return `${h}h ${m}m ${sec}s`;
}

export function HealthPage() {
	const { me } = useAuth();
	const [health, setHealth] = useState<Health | null>(null);
	const [metrics, setMetrics] = useState<Metrics | null>(null);
	const [loading, setLoading] = useState(false);
	const [err, setErr] = useState<string | null>(null);

	const reload = useCallback(async () => {
		setLoading(true);
		try {
			const h = await api<Health>("/health");
			setHealth(h);
			setErr(null);
			try {
				const m = await api<Metrics>("/metrics");
				setMetrics(m);
			} catch {
				// /metrics is admin-gated and may also be Prometheus text — fine to be null.
				setMetrics(null);
			}
		} catch (e) {
			if (e instanceof ApiError) setErr(`${e.code}: ${e.message}`);
			else setErr(e instanceof Error ? e.message : String(e));
		} finally {
			setLoading(false);
		}
	}, []);

	useEffect(() => {
		void reload();
		const id = window.setInterval(reload, 10_000);
		return () => window.clearInterval(id);
	}, [reload]);

	if (!me?.is_admin) {
		return (
			<LockedPage
				title="Health requires admin"
				description="Sign in with PYLON_ADMIN_TOKEN to view server health and metrics."
			/>
		);
	}

	const uptimeSec = health?.uptime_secs ?? health?.uptime;

	return (
		<div className="space-y-4">
			<div className="flex items-center justify-end">
				<Button size="sm" variant="outline" onClick={reload} disabled={loading}>
					{loading ? (
						<Loader2 className="size-3.5 animate-spin" />
					) : (
						<RefreshCw className="size-3.5" />
					)}
					Refresh
				</Button>
			</div>

			{err && (
				<div className="rounded-md border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive">
					{err}
				</div>
			)}

			<div className="grid gap-4 md:grid-cols-3">
				<Stat
					label="Status"
					value={
						<span
							className={
								health?.status === "ok" || health?.status === "healthy"
									? "text-emerald-500"
									: "text-amber-500"
							}
						>
							{health?.status ?? "—"}
						</span>
					}
				/>
				<Stat
					label="Uptime"
					value={uptimeSec != null ? formatUptime(uptimeSec) : "—"}
					mono
				/>
				<Stat label="Version" value={health?.version ?? "—"} mono />
			</div>

			{metrics && (
				<>
					<div className="grid gap-4 md:grid-cols-4">
						{metrics.total_requests != null && (
							<Stat
								label="Total requests"
								value={metrics.total_requests.toLocaleString()}
								mono
							/>
						)}
						{metrics.active_connections != null && (
							<Stat
								label="Active connections"
								value={metrics.active_connections}
								mono
							/>
						)}
						{metrics.error_count != null && (
							<Stat
								label="Errors"
								value={
									<span className="text-destructive">{metrics.error_count}</span>
								}
								mono
							/>
						)}
						{metrics.avg_response_ms != null && (
							<Stat
								label="Avg response"
								value={`${metrics.avg_response_ms}ms`}
								mono
							/>
						)}
					</div>

					{metrics.by_method && (
						<Card>
							<CardHeader>
								<CardTitle>Requests by method</CardTitle>
							</CardHeader>
							<CardContent>
								<div className="grid gap-2 sm:grid-cols-2 md:grid-cols-3">
									{Object.entries(metrics.by_method).map(([m, c]) => (
										<div
											key={m}
											className="flex items-center justify-between rounded-md border px-3 py-2"
										>
											<code className="text-xs">{m}</code>
											<span className="font-mono text-sm">
												{typeof c === "number" ? c.toLocaleString() : String(c)}
											</span>
										</div>
									))}
								</div>
							</CardContent>
						</Card>
					)}
				</>
			)}
		</div>
	);
}

function Stat({
	label,
	value,
	mono,
}: {
	label: string;
	value: React.ReactNode;
	mono?: boolean;
}) {
	return (
		<Card>
			<CardContent className="p-4">
				<p className="text-xs text-muted-foreground">{label}</p>
				<p
					className={`mt-1 text-xl font-semibold ${
						mono ? "font-mono" : ""
					}`}
				>
					{value}
				</p>
			</CardContent>
		</Card>
	);
}
