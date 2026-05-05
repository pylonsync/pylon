import { useEffect, useMemo, useState } from "react";
import { Toaster } from "@/components/ui/sonner";
import { AuthProvider, useAuth } from "@/auth/AuthContext";
import { StudioLayout, type StudioRoute } from "@/layout/StudioLayout";
import { resolveConfig } from "@/lib/studio-config";
import { loadExtensions, getExtensionPage } from "@/lib/extensions";
import { api, MANIFEST } from "@/lib/pylon";
import { EntitiesPage } from "@/pages/Entities";
import { FunctionsPage } from "@/pages/Functions";
import { HealthPage } from "@/pages/Health";
import { ManifestPage } from "@/pages/Manifest";
import { PoliciesPage } from "@/pages/Policies";
import { RoutesPage } from "@/pages/Routes";
import { SettingsPage } from "@/pages/Settings";
import { SyncPage } from "@/pages/Sync";
import { OverviewPage } from "@/pages/Overview";
import { RolesPage } from "@/pages/Roles";
import { ResourceListPage } from "@/pages/ResourceList";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

const config = resolveConfig();

export default function App() {
	// Default route: the Overview page. The dashboard's first hit
	// should land on the operational dashboard (live requests sparkline,
	// jobs/workflows panels) — not on whatever entity happens to be
	// first in the sidebar. Customers can pick a different default by
	// configuring sidebar.sections in studio.config.ts; if explicit,
	// honor that. Otherwise overview wins.
	const initial = useMemo<StudioRoute>(() => {
		const explicitFirstPage = config.sidebar?.sections
			?.flatMap((s) => s.items)
			.find((i) => i.type === "page");
		if (explicitFirstPage && explicitFirstPage.type === "page") {
			return { kind: "page", id: explicitFirstPage.id };
		}
		return { kind: "page", id: "overview" };
	}, []);

	const [route, setRoute] = useState<StudioRoute>(initial);
	const [extReady, setExtReady] = useState(!config.hasExtensions);

	useEffect(() => {
		if (!config.hasExtensions) return;
		void loadExtensions(true).then(() => setExtReady(true));
	}, []);

	return (
		<AuthProvider>
			<StudioLayout config={config} route={route} onRouteChange={setRoute}>
				<RouteContent route={route} extReady={extReady} />
			</StudioLayout>
			<Toaster richColors position="bottom-right" />
		</AuthProvider>
	);
}

function RouteContent({
	route,
	extReady,
}: {
	route: StudioRoute;
	extReady: boolean;
}) {
	const auth = useAuth();

	if (route.kind === "resource") {
		return <ResourceListPage entity={route.entity} config={config} />;
	}

	const id = route.id;
	switch (id) {
		case "overview":
			return <OverviewPage />;
		case "roles":
			return <RolesPage />;
		case "manifest":
			return <ManifestPage />;
		case "functions":
			return <FunctionsPage />;
		case "policies":
			return <PoliciesPage />;
		case "routes":
			return <RoutesPage />;
		case "sync":
			return <SyncPage />;
		case "health":
			return <HealthPage />;
		case "settings":
			return <SettingsPage />;
		case "entities":
			// Legacy raw-entity inspector, still useful as a debug tool.
			return <EntitiesPage />;
		default: {
			// Maybe an extension page?
			if (!extReady) {
				return (
					<div className="text-sm text-muted-foreground">Loading extensions…</div>
				);
			}
			const Custom = getExtensionPage(id);
			if (Custom) {
				return (
					<Custom
						manifest={MANIFEST}
						auth={auth}
						api={api as <T>(p: string, o?: unknown) => Promise<T>}
					/>
				);
			}
			return (
				<Card>
					<CardHeader>
						<CardTitle>Unknown page</CardTitle>
					</CardHeader>
					<CardContent className="text-sm text-muted-foreground">
						No built-in page or extension is registered for{" "}
						<code className="font-mono">{id}</code>. Check{" "}
						<code className="font-mono">studio.config.ts</code> or register
						the component in <code className="font-mono">studio.entry.tsx</code>.
					</CardContent>
				</Card>
			);
		}
	}
}
