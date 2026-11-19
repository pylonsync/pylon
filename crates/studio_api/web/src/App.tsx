import { createElement, useCallback, useEffect, useState } from "react";

import { useAuth } from "@/auth/auth";
import { AuthProvider } from "@/auth/AuthContext";
import { Page } from "@/components/page";
import { EmptyState, LoadingRows } from "@/components/states";
import { Button } from "@/components/ui/button";
import { onLinkClick } from "@/lib/navigation";
import { Toaster } from "@/components/ui/toast";
import { StudioLayout, type StudioRoute } from "@/layout/StudioLayout";
import { getExtensionPage, loadExtensions } from "@/lib/extensions";
import { humanize } from "@/lib/format";
import { api, MANIFEST } from "@/lib/pylon";
import {
	DEFAULT_PAGE,
	parseRoute,
	routeToPath,
	sameRoute,
	STUDIO_BASE,
} from "@/lib/route";
import { resolveConfig } from "@/lib/studio-config";
import { EntitiesPage } from "@/pages/Entities";
import { FunctionsPage } from "@/pages/Functions";
import { HealthPage } from "@/pages/Health";
import { JobsPage } from "@/pages/Jobs";
import { LogsPage } from "@/pages/Logs";
import { ManifestPage } from "@/pages/Manifest";
import { MetricsPage } from "@/pages/Metrics";
import { OverviewPage } from "@/pages/Overview";
import { PoliciesPage } from "@/pages/Policies";
import { ResourceListPage } from "@/pages/ResourceList";
import { RolesPage } from "@/pages/Roles";
import { RoutesPage } from "@/pages/Routes";
import { SettingsPage } from "@/pages/Settings";
import { ShardsPage } from "@/pages/Shards";
import { SyncPage } from "@/pages/Sync";
import { WorkflowsPage } from "@/pages/Workflows";

const config = resolveConfig();

/**
 * The page for a bare `/studio`: Overview, or the first page that the
 * sidebar config of the app declares.
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
	// The URL is the source of truth for the current page. A link opens the
	// same page, a reload keeps it, and Back moves inside Studio.
	const [route, setRoute] = useState<StudioRoute>(
		() => parseRoute(window.location.pathname) ?? defaultRoute(),
	);
	const [extReady, setExtReady] = useState(!config.hasExtensions);

	// The browser changes the URL before `popstate` fires, so the handler
	// reads the URL. The state object is null for entries Studio did not push.
	useEffect(() => {
		const onPop = () =>
			setRoute((current) => {
				const next = parseRoute(window.location.pathname) ?? defaultRoute();
				return sameRoute(current, next) ? current : next;
			});
		window.addEventListener("popstate", onPop);
		return () => window.removeEventListener("popstate", onPop);
	}, []);

	const navigate = useCallback((next: StudioRoute) => {
		setRoute((current) => {
			const path = routeToPath(next);
			// A click on the current item adds no history entry, unless the URL
			// has a query string to drop (an open row or shard).
			if (sameRoute(current, next)) {
				if (window.location.search) {
					window.history.pushState(null, "", path);
					window.dispatchEvent(new PopStateEvent("popstate"));
				}
				return current;
			}
			window.history.pushState(null, "", path);
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
			<Toaster />
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
		return (
			<ResourceListPage key={route.entity} entity={route.entity} config={config} />
		);
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
		case "logs":
			return <LogsPage />;
		case "metrics":
			return <MetricsPage />;
		case "jobs":
			return <JobsPage />;
		case "workflows":
			return <WorkflowsPage />;
		case "shards":
			return <ShardsPage />;
		case "settings":
			return <SettingsPage />;
		case "entities":
			return <EntitiesPage />;
		default: {
			const title = config.sidebar?.sections
				?.flatMap((s) => s.items)
				.find((i) => i.type === "page" && i.id === id);
			const label =
				title && title.type === "page" ? title.label : humanize(id);
			if (!extReady) {
				return (
					<Page title={label}>
						<LoadingRows />
					</Page>
				);
			}
			const custom = getExtensionPage(id);
			if (custom) {
				return (
					<Page title={label}>
						<div className="min-h-0 flex-1 overflow-auto p-3">
							{createElement(custom, {
								manifest: MANIFEST,
								auth,
								api: api as <T>(p: string, o?: unknown) => Promise<T>,
							})}
						</div>
					</Page>
				);
			}
			return (
				<Page title="Page not found">
					<EmptyState
						title={`Studio has no page "${id}"`}
						description={
							<>
								Add the page to <code>studio.config.ts</code> and register its
								component in <code>studio.entry.tsx</code>, or select a page
								from the sidebar.
							</>
						}
						action={
							<Button
								variant="outline"
								size="sm"
								render={
									<a
										href={STUDIO_BASE}
										onClick={(e) => onLinkClick(e, STUDIO_BASE)}
									/>
								}
							>
								Go to Overview
							</Button>
						}
					/>
				</Page>
			);
		}
	}
}
