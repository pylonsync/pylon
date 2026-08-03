import { useCallback, useEffect, useState } from "react";
import { Toaster } from "@/components/ui/sonner";
import { AuthProvider, useAuth } from "@/auth/AuthContext";
import { StudioLayout, type StudioRoute } from "@/layout/StudioLayout";
import { DEFAULT_PAGE, parseRoute, routeToPath, sameRoute } from "@/lib/route";
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

/**
 * Where a bare `/studio` lands. The operational dashboard, not whatever entity
 * happens to sort first in the sidebar — unless the app's `studio.config.ts`
 * declares its own first page, in which case that was an explicit choice.
 */
function defaultRoute(): StudioRoute {
	const explicitFirstPage = config.sidebar?.sections
		?.flatMap((s) => s.items)
		.find((i) => i.type === "page");
	if (explicitFirstPage && explicitFirstPage.type === "page") {
		return { kind: "page", id: explicitFirstPage.id };
	}
	return { kind: "page", id: DEFAULT_PAGE };
}

export default function App() {
	// The URL is the source of truth, not component state. Deep links work,
	// refresh keeps your place, and the back button moves within Studio
	// instead of leaving it.
	const [route, setRoute] = useState<StudioRoute>(
		() => parseRoute(window.location.pathname) ?? defaultRoute(),
	);
	const [extReady, setExtReady] = useState(!config.hasExtensions);

	// Back/forward. The browser has already changed the URL by the time this
	// fires, so read it rather than trusting popstate's state object — which
	// is null for entries we didn't push (the initial load, most notably).
	useEffect(() => {
		const onPop = () =>
			setRoute(parseRoute(window.location.pathname) ?? defaultRoute());
		window.addEventListener("popstate", onPop);
		return () => window.removeEventListener("popstate", onPop);
	}, []);

	const navigate = useCallback((next: StudioRoute) => {
		setRoute((current) => {
			// Re-clicking the current item shouldn't stack history entries that
			// do nothing when you press back.
			if (sameRoute(current, next)) return current;
			window.history.pushState(null, "", routeToPath(next));
			return next;
		});
	}, []);

	useEffect(() => {
		if (!config.hasExtensions) return;
		void loadExtensions(true).then(() => setExtReady(true));
	}, []);

	return (
		<AuthProvider>
			<StudioLayout config={config} route={route} onRouteChange={navigate}>
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
