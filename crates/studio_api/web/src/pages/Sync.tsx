import { useEffect, useRef, useState } from "react";
import { Pause, Play, Trash2 } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { API_BASE } from "@/lib/pylon";

type Event = {
	seq: number;
	kind: string;
	entity: string;
	row_id: string;
	timestamp: string;
};

function kindBadge(kind: string) {
	switch (kind) {
		case "insert":
			return "default" as const;
		case "update":
			return "secondary" as const;
		case "delete":
			return "destructive" as const;
		default:
			return "outline" as const;
	}
}

export function SyncPage() {
	const [events, setEvents] = useState<Event[]>([]);
	const [paused, setPaused] = useState(false);
	const [connected, setConnected] = useState(false);
	const wsRef = useRef<WebSocket | null>(null);
	const pausedRef = useRef(paused);
	useEffect(() => {
		pausedRef.current = paused;
	}, [paused]);

	useEffect(() => {
		try {
			// WebSocket port = HTTP port + 1 by Pylon convention.
			const url = new URL(API_BASE || window.location.origin);
			const wsPort = parseInt(url.port || "4321", 10) + 1;
			const proto = url.protocol === "https:" ? "wss:" : "ws:";
			const ws = new WebSocket(`${proto}//${url.hostname}:${wsPort}`);
			wsRef.current = ws;
			ws.onopen = () => setConnected(true);
			ws.onclose = () => setConnected(false);
			ws.onerror = () => setConnected(false);
			ws.onmessage = (e) => {
				if (pausedRef.current) return;
				try {
					const msg = JSON.parse(e.data);
					if (msg.seq != null && msg.kind && msg.entity) {
						setEvents((prev) =>
							[
								{
									seq: msg.seq,
									kind: msg.kind,
									entity: msg.entity,
									row_id: msg.row_id ?? msg.id ?? "—",
									timestamp: new Date().toISOString(),
								},
								...prev,
							].slice(0, 200),
						);
					}
				} catch {
					// ignore non-JSON pings/etc.
				}
			};
			return () => {
				ws.close();
			};
		} catch {
			// Bad URL or browser blocked the connection — non-fatal, just no live data.
		}
	}, []);

	return (
		<div className="space-y-4">
			<div className="flex items-center gap-2">
				<Badge variant={connected ? "default" : "outline"}>
					<span
						className={`mr-1.5 inline-block size-1.5 rounded-full ${
							connected ? "bg-emerald-500" : "bg-zinc-500"
						}`}
					/>
					{connected ? "Connected" : "Disconnected"}
				</Badge>
				<span className="text-xs text-muted-foreground">
					{events.length} event{events.length === 1 ? "" : "s"} (max 200)
				</span>
				<div className="ml-auto flex gap-2">
					<Button
						size="sm"
						variant="outline"
						onClick={() => setPaused((p) => !p)}
					>
						{paused ? (
							<>
								<Play className="size-3.5" /> Resume
							</>
						) : (
							<>
								<Pause className="size-3.5" /> Pause
							</>
						)}
					</Button>
					<Button
						size="sm"
						variant="ghost"
						onClick={() => setEvents([])}
						disabled={events.length === 0}
					>
						<Trash2 className="size-3.5" /> Clear
					</Button>
				</div>
			</div>

			<Card>
				<CardContent className="p-0">
					{events.length === 0 ? (
						<div className="py-16 text-center text-sm text-muted-foreground">
							No events yet. Make a change in the Entities tab to see one
							appear here in real time.
						</div>
					) : (
						<div className="divide-y font-mono text-xs">
							{events.map((e, i) => (
								<div
									key={`${e.seq}-${i}`}
									className="flex items-center gap-3 px-4 py-2"
								>
									<span className="text-muted-foreground">[{e.seq}]</span>
									<Badge variant={kindBadge(e.kind)} className="w-16 justify-center">
										{e.kind}
									</Badge>
									<code className="text-sm">{e.entity}</code>
									<span className="text-muted-foreground">/</span>
									<code>{e.row_id}</code>
									<span className="ml-auto text-muted-foreground">
										{new Date(e.timestamp).toLocaleTimeString()}
									</span>
								</div>
							))}
						</div>
					)}
				</CardContent>
			</Card>
		</div>
	);
}
