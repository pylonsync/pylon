import { displayName, useAuth } from "@/auth/auth";
import { Fact, Facts } from "@/components/facts";
import { Page, PageScroll, Section } from "@/components/page";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Select } from "@/components/ui/select";
import { usePreference } from "@ui-sdk/react/preferences";
import { API_BASE, MANIFEST } from "@/lib/pylon";
import { colorScheme, type ColorSchemeChoice } from "@/lib/theme";

export function SettingsPage() {
	const auth = useAuth();
	const { me } = auth;
	const { value: scheme } = usePreference(colorScheme);
	return (
		<Page title="Settings">
			<PageScroll>
				<Section title="Session">
					<Facts className="max-w-xl">
						<Fact label="Signed in as">{displayName(auth)}</Fact>
						<Fact label="Account type">
							{auth.operator ? "Studio operator" : "App user"}
						</Fact>
						<Fact label="User id">
							<code className="text-xs">{me?.user_id ?? "none"}</code>
						</Fact>
						<Fact label="Admin">
							<Badge variant={me?.is_admin ? "green" : "gray"}>
								{me?.is_admin ? "Yes" : "No"}
							</Badge>
						</Fact>
						{me?.tenant_id && (
							<Fact label="Tenant">
								<code className="text-xs">{me.tenant_id}</code>
							</Fact>
						)}
						<Fact label="">
							<Button variant="outline" render={<a href="/studio/logout" />}>
								Sign out
							</Button>
						</Fact>
					</Facts>
				</Section>
				<Section title="Appearance">
					<Facts className="max-w-xl">
						<Fact label="Theme">
							<Select
								aria-label="Theme"
								className="w-40"
								value={scheme}
								onValueChange={(v) => colorScheme.set(v as ColorSchemeChoice)}
								options={[
									{ value: "system", label: "System" },
									{ value: "light", label: "Light" },
									{ value: "dark", label: "Dark" },
								]}
							/>
						</Fact>
					</Facts>
				</Section>
				<Section title="Server">
					<Facts className="max-w-xl">
						<Fact label="API base">
							<code className="text-xs">{API_BASE || "same origin"}</code>
						</Fact>
						<Fact label="App">
							<code className="text-xs">{MANIFEST.name}</code>
						</Fact>
						<Fact label="App version">
							<code className="text-xs">{MANIFEST.version}</code>
						</Fact>
						<Fact label="Manifest version">
							<code className="text-xs">{MANIFEST.manifest_version}</code>
						</Fact>
					</Facts>
				</Section>
			</PageScroll>
		</Page>
	);
}
