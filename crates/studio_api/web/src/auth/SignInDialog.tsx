import { useState } from "react";
import { Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ApiError } from "@/lib/pylon";
import { useAuth } from "./AuthContext";

export function SignInDialog({
	open,
	onOpenChange,
}: {
	open: boolean;
	onOpenChange: (open: boolean) => void;
}) {
	const { signIn } = useAuth();
	const [token, setToken] = useState("");
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string | null>(null);

	async function onSubmit(e: React.FormEvent) {
		e.preventDefault();
		setError(null);
		setBusy(true);
		try {
			// Cookie path: POST to /studio/login. Server verifies the
			// token, sets an HttpOnly admin cookie, returns 303. We
			// fetch with redirect:"manual" so we can read non-2xx as a
			// useful error and keep the dialog open. On success the
			// cookie is set; reload so every Studio data fetch picks
			// up the new auth without threading an "auth changed"
			// event through every component.
			//
			// Falls back to the legacy Bearer + localStorage path if
			// /studio/login 404s — keeps Studio usable against older
			// pylon versions.
			const body = new URLSearchParams({ token: token.trim() });
			const res = await fetch("/studio/login", {
				method: "POST",
				body,
				headers: { "Content-Type": "application/x-www-form-urlencoded" },
				credentials: "include",
				redirect: "manual",
			});
			if (res.status === 303 || res.ok) {
				setToken("");
				onOpenChange(false);
				window.location.reload();
				return;
			}
			if (res.status === 404) {
				await signIn(token.trim());
				setToken("");
				onOpenChange(false);
				return;
			}
			if (res.status === 401) {
				setError("Invalid admin token.");
				return;
			}
			setError(`Sign-in failed (HTTP ${res.status}).`);
		} catch (err) {
			if (err instanceof ApiError) {
				setError(`${err.code}: ${err.message}`);
			} else {
				setError(err instanceof Error ? err.message : String(err));
			}
		} finally {
			setBusy(false);
		}
	}

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="sm:max-w-[480px]">
				<DialogHeader>
					<DialogTitle>Sign in to Studio</DialogTitle>
					<DialogDescription>
						Paste your <code className="rounded bg-muted px-1 py-0.5 text-xs">PYLON_ADMIN_TOKEN</code>{" "}
						to access admin tabs (functions, jobs, scheduler, etc.). Token is
						stored in your browser's localStorage and sent as a Bearer header
						with every request.
					</DialogDescription>
				</DialogHeader>
				<form onSubmit={onSubmit} className="space-y-4">
					<div className="space-y-2">
						<Label htmlFor="admin-token">Admin token</Label>
						<Input
							id="admin-token"
							type="password"
							autoFocus
							autoComplete="off"
							placeholder="pln_admin_…"
							value={token}
							onChange={(e) => setToken(e.target.value)}
						/>
						<p className="text-xs text-muted-foreground">
							Set on the server with{" "}
							<code className="rounded bg-muted px-1 py-0.5">
								PYLON_ADMIN_TOKEN=…
							</code>
							. In dev, any token Pylon validates as admin works.
						</p>
					</div>
					{error && (
						<div className="rounded-md border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive">
							{error}
						</div>
					)}
					<DialogFooter>
						<Button
							type="button"
							variant="ghost"
							onClick={() => onOpenChange(false)}
							disabled={busy}
						>
							Cancel
						</Button>
						<Button type="submit" disabled={busy || !token.trim()}>
							{busy ? (
								<>
									<Loader2 className="size-4 animate-spin" /> Signing in…
								</>
							) : (
								"Sign in"
							)}
						</Button>
					</DialogFooter>
				</form>
			</DialogContent>
		</Dialog>
	);
}
