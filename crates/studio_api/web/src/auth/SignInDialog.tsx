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
			await signIn(token.trim());
			setToken("");
			onOpenChange(false);
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
