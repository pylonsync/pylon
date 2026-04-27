import { useCallback, useEffect, useState } from "react";
import { Loader2, Play, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import {
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
} from "@/components/ui/tabs";
import { ApiError, api } from "@/lib/pylon";
import { toast } from "sonner";
import { LockedPage } from "./Locked";
import { useAuth } from "@/auth/AuthContext";

type FnDef = {
	name: string;
	fn_type?: string;
};

type Trace = {
	fn_name?: string;
	name?: string;
	timestamp?: string;
	duration_ms?: number;
	error?: string | null;
};

export function FunctionsPage() {
	const { me } = useAuth();
	const [fns, setFns] = useState<FnDef[]>([]);
	const [traces, setTraces] = useState<Trace[]>([]);
	const [loading, setLoading] = useState(false);
	const [name, setName] = useState("");
	const [argsJson, setArgsJson] = useState("{}");
	const [result, setResult] = useState<{ status: number; data: unknown } | null>(
		null,
	);
	const [invoking, setInvoking] = useState(false);

	const reload = useCallback(async () => {
		setLoading(true);
		try {
			const [fnsData, tracesData] = await Promise.all([
				api<FnDef[] | { data?: FnDef[] }>("/api/fn"),
				api<Trace[] | { data?: Trace[] }>("/api/fn/traces"),
			]);
			setFns(Array.isArray(fnsData) ? fnsData : fnsData.data ?? []);
			setTraces(Array.isArray(tracesData) ? tracesData : tracesData.data ?? []);
		} catch (err) {
			if (err instanceof ApiError && (err.status === 401 || err.status === 403)) {
				// Locked page handles this; don't toast.
			} else if (err instanceof ApiError) {
				toast.error(`${err.code}: ${err.message}`);
			} else {
				toast.error(err instanceof Error ? err.message : String(err));
			}
		} finally {
			setLoading(false);
		}
	}, []);

	useEffect(() => {
		if (!me?.is_admin) return;
		void reload();
		const id = window.setInterval(reload, 5000);
		return () => window.clearInterval(id);
	}, [reload, me?.is_admin]);

	if (!me?.is_admin) {
		return (
			<LockedPage
				title="Functions require admin"
				description="Sign in with PYLON_ADMIN_TOKEN to enumerate registered functions and view traces."
			/>
		);
	}

	const invoke = async () => {
		if (!name.trim()) return;
		setInvoking(true);
		try {
			const args = argsJson.trim() ? JSON.parse(argsJson) : {};
			const data = await api(`/api/fn/${name.trim()}`, {
				method: "POST",
				body: JSON.stringify(args),
			});
			setResult({ status: 200, data });
			toast.success(`Called ${name}`);
			void reload();
		} catch (err) {
			if (err instanceof ApiError) {
				setResult({ status: err.status, data: { code: err.code, message: err.message } });
			} else {
				setResult({
					status: 0,
					data: { error: err instanceof Error ? err.message : String(err) },
				});
			}
		} finally {
			setInvoking(false);
		}
	};

	return (
		<div className="space-y-4">
			<Tabs defaultValue="registered">
				<div className="flex items-center justify-between">
					<TabsList>
						<TabsTrigger value="registered">Registered ({fns.length})</TabsTrigger>
						<TabsTrigger value="traces">Traces ({traces.length})</TabsTrigger>
					</TabsList>
					<Button variant="outline" size="sm" onClick={reload} disabled={loading}>
						{loading ? (
							<Loader2 className="size-3.5 animate-spin" />
						) : (
							<RefreshCw className="size-3.5" />
						)}
						Refresh
					</Button>
				</div>

				<TabsContent value="registered" className="grid gap-4 md:grid-cols-2">
					<Card>
						<CardHeader>
							<CardTitle>Registered functions</CardTitle>
							<CardDescription>
								TypeScript functions discovered in your <code>functions/</code> dir.
							</CardDescription>
						</CardHeader>
						<CardContent className="space-y-1">
							{fns.length === 0 ? (
								<p className="text-sm text-muted-foreground">
									No functions registered. Add files to{" "}
									<code className="rounded bg-muted px-1 py-0.5">./functions/</code>{" "}
									and restart.
								</p>
							) : (
								fns.map((f) => (
									<div
										key={f.name}
										className="flex items-center justify-between rounded-md border px-3 py-2"
									>
										<div className="flex items-center gap-2">
											<code className="text-xs">{f.name}</code>
											{f.fn_type && (
												<Badge variant="secondary" className="text-[10px]">
													{f.fn_type}
												</Badge>
											)}
										</div>
										<Button
											size="sm"
											variant="ghost"
											onClick={() => {
												setName(f.name);
												setArgsJson("{}");
												setResult(null);
											}}
										>
											<Play className="size-3.5" /> Invoke
										</Button>
									</div>
								))
							)}
						</CardContent>
					</Card>

					<Card>
						<CardHeader>
							<CardTitle>Invoke</CardTitle>
							<CardDescription>POST to /api/fn/&lt;name&gt; with the JSON body below.</CardDescription>
						</CardHeader>
						<CardContent className="space-y-3">
							<div className="space-y-1.5">
								<Label htmlFor="fn-name">Function name</Label>
								<Input
									id="fn-name"
									className="font-mono"
									placeholder="myFunction"
									value={name}
									onChange={(e) => setName(e.target.value)}
								/>
							</div>
							<div className="space-y-1.5">
								<Label htmlFor="fn-args">Arguments (JSON)</Label>
								<Textarea
									id="fn-args"
									className="font-mono text-xs"
									rows={6}
									value={argsJson}
									onChange={(e) => setArgsJson(e.target.value)}
								/>
							</div>
							<Button onClick={invoke} disabled={!name.trim() || invoking}>
								{invoking && <Loader2 className="size-3.5 animate-spin" />}
								Call
							</Button>
							{result && (
								<div className="space-y-1.5">
									<div className="flex items-center gap-2 text-xs">
										<Badge
											variant={
												result.status >= 200 && result.status < 300
													? "default"
													: "destructive"
											}
										>
											{result.status || "ERR"}
										</Badge>
									</div>
									<pre className="max-h-64 overflow-auto rounded-md border bg-muted/30 p-3 text-xs">
										{JSON.stringify(result.data, null, 2)}
									</pre>
								</div>
							)}
						</CardContent>
					</Card>
				</TabsContent>

				<TabsContent value="traces">
					<Card>
						<CardContent className="p-0">
							{traces.length === 0 ? (
								<div className="py-16 text-center text-sm text-muted-foreground">
									No traces yet. Invoke a function to see one here.
								</div>
							) : (
								<div className="divide-y">
									{traces.map((t, i) => (
										<details key={i} className="group">
											<summary className="flex cursor-pointer items-center justify-between gap-2 px-4 py-2.5 hover:bg-accent/40">
												<div className="flex items-center gap-2">
													<code className="text-xs">{t.fn_name ?? t.name}</code>
													<Badge variant={t.error ? "destructive" : "default"}>
														{t.error ? "error" : "ok"}
													</Badge>
												</div>
												<span className="text-xs text-muted-foreground">
													{t.duration_ms != null ? `${t.duration_ms}ms` : ""}
													{t.timestamp ? ` · ${new Date(t.timestamp).toLocaleTimeString()}` : ""}
												</span>
											</summary>
											<pre className="max-h-64 overflow-auto bg-muted/30 px-4 py-3 text-xs">
												{JSON.stringify(t, null, 2)}
											</pre>
										</details>
									))}
								</div>
							)}
						</CardContent>
					</Card>
				</TabsContent>
			</Tabs>
		</div>
	);
}
