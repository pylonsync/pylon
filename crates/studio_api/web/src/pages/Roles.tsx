import { Page, PageScroll, Section } from "@/components/page";

export function RolesPage() {
	return (
		<Page title="Roles">
			<PageScroll>
				<Section title="Admin">
					<div className="max-w-2xl space-y-2 px-3 pb-3 text-[13px] text-muted-foreground">
						<p>
							<code>is_admin</code> is the only built-in role. A user is an admin
							when the field named by <code>auth.user.adminField</code> is true
							on the User row, or when the email is in{" "}
							<code>PYLON_ADMIN_EMAILS</code>.
						</p>
					</div>
				</Section>
				<Section title="Custom roles">
					<div className="max-w-2xl space-y-2 px-3 pb-3 text-[13px] text-muted-foreground">
						<p>
							Store the list of roles on the User row and name the field in{" "}
							<code>auth.user.expose</code> in <code>app.ts</code>. The auth
							context gives the list to policies as <code>auth.roles</code>.
						</p>
						<p>
							A policy tests a role with{" "}
							<code>auth.roles.includes("editor")</code>.
						</p>
					</div>
				</Section>
			</PageScroll>
		</Page>
	);
}
