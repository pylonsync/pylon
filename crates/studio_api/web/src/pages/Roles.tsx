import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";

export function RolesPage() {
	return (
		<div className="space-y-6">
			<div>
				<h1 className="text-2xl font-semibold tracking-tight">Roles & Permissions</h1>
				<p className="mt-1 text-sm text-muted-foreground">
					Pylon enforces row-level access control through manifest policies. The
					table below summarises the configured rules.
				</p>
			</div>
			<Card>
				<CardHeader>
					<CardTitle className="text-base">Role primitives</CardTitle>
					<CardDescription>
						`is_admin` is the only built-in role. It is granted per user, by
						`auth.user.adminField` on the User row or the `PYLON_ADMIN_EMAILS`
						allowlist. Custom roles attach through the `roles` claim on the
						auth context — set them in your auth provider.
					</CardDescription>
				</CardHeader>
				<CardContent className="text-sm text-muted-foreground space-y-2">
					<p>
						To attach roles per-user: configure `auth.user.expose` in
						`app.ts`, then store the list on the User row. The auth context
						reads the field and exposes it as `auth.roles`.
					</p>
					<p>
						Reference the role from a policy with{" "}
						<code className="rounded bg-muted px-1 py-0.5">
							auth.roles.includes("editor")
						</code>
						.
					</p>
				</CardContent>
			</Card>
		</div>
	);
}
