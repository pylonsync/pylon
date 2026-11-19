import { useState } from "react";
import { Play, RefreshCw } from "lucide-react";

import { useAuth } from "@/auth/auth";
import { CodeBlock } from "@/components/code-block";
import { Page } from "@/components/page";
import { Dash } from "@/components/renderers";
import {
	EmptyState,
	ErrorState,
	LoadingRows,
	Spinner,
} from "@/components/states";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "@/components/ui/table";
import { Tabs, TabsList, TabsPanel, TabsTab } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { Tooltip } from "@/components/ui/tooltip";
import { useQuery } from "@/hooks/use-query";
import { errorText, formatTime, parseDate } from "@/lib/format";
import { ApiError, api } from "@/lib/pylon";
import { cn } from "@/lib/utils";
import { LockedPage } from "./Locked";

type ArgSchema = { type?: string; optional?: boolean; table?: string };

type FnDef = {
	name: string;
	fn_type?: string;
	auth?: string;
	internal?: boolean;
	args_schema?: Record<string, ArgSchema>;
};

type Trace = {
	fn_name?: string;
	name?: string;
	timestamp?: string;
	duration_ms?: number;
	error?: string | null;
};

type Result = { ok: boolean; status: number; data: unknown };

const POLL_MS = 5000;

/** A JSON body with one placeholder for each required argument. */
function argsTemplate(fn: FnDef): string {
	const out: Record<string, string> = {};
	for (const [name, arg] of Object.entries(fn.args_schema ?? {})) {
		if (arg.optional) continue;
		out[name] = `<${arg.type === "id" && arg.table ? `id(${arg.table})` : (arg.type ?? "value")}>`;
	}
	return JSON.stringify(out, null, 2);
}

function signature(fn: FnDef): string {
	return Object.entries(fn.args_schema ?? {})
		.map(
			([name, arg]) =>
				`${name}${arg.optional ? "?" : ""}: ${arg.type === "id" && arg.table ? `id(${arg.table})` : (arg.type ?? "unknown")}`,
		)
		.join(", ");
}

export function FunctionsPage() {
	const { me } = useAuth();
	const isAdmin = !!me?.is_admin;
	const { data, error, loading, reload } = useQuery(
		"functions",
		async () => {
			const [fnsData, tracesData] = await Promise.all([
				api<FnDef[] | { data?: FnDef[] }>("/api/fn"),
				api<Trace[] | { data?: Trace[] }>("/api/fn/traces"),
			]);
			return {
				fns: Array.isArray(fnsData) ? fnsData : (fnsData?.data ?? []),
				traces: Array.isArray(tracesData)
					? tracesData
					: (tracesData?.data ?? []),
			};
		},
		{ pollMs: POLL_MS, enabled: isAdmin },
	);
	const fns = data?.fns ?? null;
	const traces = data?.traces ?? [];
	const [name, setName] = useState("");
	const [argsJson, setArgsJson] = useState("{}");
	const [result, setResult] = useState<Result | null>(null);
	const [tab, setTab] = useState("registered");

	if (!isAdmin) {
		return (
			<Page title="Functions">
				<LockedPage
					title="Functions are for admins"
					description="Sign in with an admin account to see the functions and their traces."
				/>
			</Page>
		);
	}

	const invoke = async () => {
		const fn = name.trim();
		if (!fn) return;
		let args: unknown;
		try {
			args = argsJson.trim() ? JSON.parse(argsJson) : {};
		} catch (err) {
			setResult({
				ok: false,
				status: 0,
				data: { error: `The arguments are not valid JSON: ${errorText(err)}` },
			});
			return;
		}
		try {
			const data = await api(`/api/fn/${encodeURIComponent(fn)}`, {
				method: "POST",
				body: JSON.stringify(args),
			});
			setResult({ ok: true, status: 200, data });
			void reload();
		} catch (err) {
			setResult(
				err instanceof ApiError
					? {
							ok: false,
							status: err.status,
							data: { code: err.code, message: err.message },
						}
					: { ok: false, status: 0, data: { error: errorText(err) } },
			);
		}
	};

	return (
		<Page
			title="Functions"
			actions={
				<Tooltip label="Load the functions and traces again">
					<Button
						variant="ghost"
						size="icon"
						aria-label="Refresh"
						disabled={loading}
						onClick={reload}
					>
						{loading ? <Spinner /> : <RefreshCw />}
					</Button>
				</Tooltip>
			}
		>
			<Tabs value={tab} onValueChange={(v) => v && setTab(v)} className="flex-1">
				<TabsList aria-label="Function views">
					<TabsTab value="registered">
						Registered
						<span className="text-xs text-muted-foreground tabular-nums">
							{fns?.length ?? 0}
						</span>
					</TabsTab>
					<TabsTab value="traces">
						Traces
						<span className="text-xs text-muted-foreground tabular-nums">
							{traces.length}
						</span>
					</TabsTab>
				</TabsList>

				<TabsPanel
					value="registered"
					className="flex min-h-0 flex-col lg:flex-row"
				>
					<div className="min-h-0 min-w-0 flex-1 overflow-auto max-lg:max-h-[45%] max-lg:border-b lg:border-r">
						{error && fns === null ? (
							<ErrorState
								title="Studio could not load the functions"
								message={error}
								onRetry={reload}
							/>
						) : fns === null ? (
							<LoadingRows />
						) : fns.length === 0 ? (
							<EmptyState
								title="The app has no functions"
								description={
									<>
										Add a file to the <code>functions/</code> directory and
										start the server again.
									</>
								}
							/>
						) : (
							<Table aria-label="Registered functions">
								<TableHeader>
									<tr>
										<TableHead className="w-64">Name</TableHead>
										<TableHead className="w-24">Type</TableHead>
										<TableHead className="w-20">Auth</TableHead>
										<TableHead>Arguments</TableHead>
									</tr>
								</TableHeader>
								<TableBody>
									{fns.map((f) => (
										<TableRow
											key={f.name}
											tabIndex={0}
											data-current={f.name === name}
											className="cursor-default"
											onClick={() => {
												setName(f.name);
												setArgsJson(argsTemplate(f));
												setResult(null);
											}}
											onKeyDown={(e) => {
												if (e.key !== "Enter" && e.key !== " ") return;
												e.preventDefault();
												e.currentTarget.click();
											}}
										>
											<TableCell className="font-mono text-xs">
												{f.name}
												{f.internal && (
													<Badge variant="outline" className="ml-2">
														internal
													</Badge>
												)}
											</TableCell>
											<TableCell>
												{f.fn_type ? (
													<Badge
														variant={
															f.fn_type === "query"
																? "blue"
																: f.fn_type === "mutation"
																	? "green"
																	: "amber"
														}
													>
														{f.fn_type}
													</Badge>
												) : (
													<Dash />
												)}
											</TableCell>
											<TableCell className="text-muted-foreground">
												{f.auth ?? <Dash />}
											</TableCell>
											<TableCell className="font-mono text-xs text-muted-foreground">
												{signature(f) || <Dash />}
											</TableCell>
										</TableRow>
									))}
								</TableBody>
							</Table>
						)}
					</div>

					<form
						className="flex min-h-0 w-full shrink-0 flex-col gap-3 overflow-auto p-3 lg:w-[380px]"
						onSubmit={(e) => {
							e.preventDefault();
						}}
					>
						<h2 className="text-xs font-semibold">Call a function</h2>
						<div className="space-y-1">
							<Label htmlFor="fn-name">Function</Label>
							<Input
								id="fn-name"
								name="function"
								className="font-mono text-xs"
								placeholder="Select a function from the list"
								value={name}
								onChange={(e) => setName(e.target.value)}
							/>
						</div>
						<div className="space-y-1">
							<Label htmlFor="fn-args">Arguments (JSON)</Label>
							<Textarea
								id="fn-args"
								name="arguments"
								className="font-mono text-xs leading-relaxed"
								rows={7}
								spellCheck={false}
								value={argsJson}
								onChange={(e) => setArgsJson(e.target.value)}
							/>
						</div>
						<div>
							<Button type="button" disabled={!name.trim()} onClick={invoke}>
								<Play />
								Call
							</Button>
						</div>
						{result && (
							<div className="space-y-1.5">
								<Badge variant={result.ok ? "green" : "red"}>
									{result.status
										? `${result.status} ${result.ok ? "OK" : "Error"}`
										: "Error"}
								</Badge>
								<CodeBlock
									className="max-h-72"
									value={JSON.stringify(result.data, null, 2) ?? "null"}
								/>
							</div>
						)}
					</form>
				</TabsPanel>

				<TabsPanel value="traces" className="overflow-auto">
					{error && (
						<ErrorState
							title="Studio could not load the traces"
							message={error}
							onRetry={reload}
						/>
					)}
					{!error && traces.length === 0 && (
						<EmptyState
							title="No traces"
							description="Call a function to record a trace."
						/>
					)}
					{!error &&
						traces.map((t, i) => {
							const at = parseDate(t.timestamp);
							return (
								<details key={i} className="group border-b">
									<summary
										className={cn(
											"flex h-8 cursor-default list-none items-center gap-3 px-3 hover:bg-muted [&::-webkit-details-marker]:hidden",
										)}
									>
										<Badge variant={t.error ? "red" : "green"}>
											{t.error ? "error" : "ok"}
										</Badge>
										<code className="min-w-0 flex-1 truncate text-xs">
											{t.fn_name ?? t.name}
										</code>
										<span className="text-xs text-muted-foreground tabular-nums">
											{t.duration_ms != null ? `${t.duration_ms} ms` : ""}
										</span>
										<span className="w-20 text-right text-xs text-muted-foreground tabular-nums">
											{at ? formatTime(at) : ""}
										</span>
									</summary>
									<div className="border-t bg-muted/40 p-3">
										<CodeBlock
											className="max-h-72 bg-background"
											value={JSON.stringify(t, null, 2)}
										/>
									</div>
								</details>
							);
						})}
				</TabsPanel>
			</Tabs>
		</Page>
	);
}
