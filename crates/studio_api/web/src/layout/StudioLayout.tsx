import { useCallback, useEffect, useMemo, useState } from "react";

import { Sheet, SheetContent, SheetTitle } from "@/components/ui/sheet";
import { useAuth } from "@/auth/auth";
import { NARROW_QUERY, useMediaQuery } from "@/hooks/use-media-query";
import { ShellContext, type Shell } from "@/layout/shell";
import { Sidebar } from "@/layout/Sidebar";
import { ThemeProvider } from "@/layout/ThemeProvider";
import { resolveNav } from "@/layout/resolve-nav";
import { loadCounts } from "@/lib/counts";
import { MANIFEST } from "@/lib/pylon";
import type { StudioConfig } from "@/lib/studio-config";
import { LockedPage } from "@/pages/Locked";

/// What the operator is looking at: a page, or the list of one entity.
export type StudioRoute =
	| { kind: "page"; id: string }
	| { kind: "resource"; entity: string };

const COLLAPSED_KEY = "pylon-studio-sidebar-collapsed";

function readCollapsed(): boolean {
	try {
		return window.localStorage.getItem(COLLAPSED_KEY) === "1";
	} catch {
		return false;
	}
}

/**
 * The frame of Studio: a 220px sidebar on the left and the page on the right.
 * Under 768px the sidebar is a sheet that the button in the top bar opens.
 */
export function StudioLayout({
	config,
	route,
	onRouteChange,
	children,
}: {
	config: StudioConfig;
	route: StudioRoute;
	onRouteChange: (r: StudioRoute) => void;
	children: React.ReactNode;
}) {
	const { me, loading } = useAuth();
	const isAdmin = !!me?.is_admin;
	const narrow = useMediaQuery(NARROW_QUERY);
	const collapsible = config.sidebar?.collapsible !== false;
	const [collapsed, setCollapsed] = useState(readCollapsed);
	const [sheetOpen, setSheetOpen] = useState(false);

	const sections = useMemo(() => resolveNav(config, MANIFEST), [config]);

	// A wide window has no sheet. Close it so it is not open after a resize.
	if (!narrow && sheetOpen) setSheetOpen(false);

	useEffect(() => {
		if (!isAdmin) return;
		const entities = sections.flatMap((s) =>
			s.items.flatMap((i) => (i.kind === "resource" ? [i.entity] : [])),
		);
		void loadCounts(entities);
	}, [isAdmin, sections]);

	const toggleNav = useCallback(() => {
		if (narrow) {
			setSheetOpen(true);
			return;
		}
		setCollapsed((was) => {
			const next = !was;
			try {
				window.localStorage.setItem(COLLAPSED_KEY, next ? "1" : "0");
			} catch {
				// The choice lasts for this page load only.
			}
			return next;
		});
	}, [narrow]);

	const hidden = narrow || (collapsible && collapsed);
	const shell = useMemo<Shell>(
		() => ({
			navHidden: hidden,
			navToggle: narrow || collapsible,
			toggleNav,
			navigate: onRouteChange,
		}),
		[hidden, narrow, collapsible, toggleNav, onRouteChange],
	);

	// The server serves Studio to a signed-in admin only. A session without
	// admin here ended while the tab was open, so the page says that in place
	// of a list of failed requests.
	const sessionLost = !loading && !isAdmin;

	return (
		<ThemeProvider theme={config.theme}>
			<ShellContext.Provider value={shell}>
				<div className="flex h-dvh min-h-0 w-full overflow-hidden">
					{!hidden && (
						<aside className="w-[220px] shrink-0 border-r">
							<Sidebar config={config} sections={sections} route={route} />
						</aside>
					)}
					{narrow && (
						<Sheet open={sheetOpen} onOpenChange={setSheetOpen}>
							<SheetContent
								side="left"
								initialFocus="content"
								className="w-[min(260px,85vw)]"
							>
								<SheetTitle className="sr-only">Navigation</SheetTitle>
								<Sidebar
									config={config}
									sections={sections}
									route={route}
									onNavigate={() => setSheetOpen(false)}
								/>
							</SheetContent>
						</Sheet>
					)}
					<main className="flex min-h-0 min-w-0 flex-1 flex-col">
						{sessionLost ? (
							<LockedPage
								title="Your session ended"
								description="Sign in again to continue."
								action={{ label: "Sign in", href: "/studio/logout" }}
							/>
						) : (
							children
						)}
					</main>
				</div>
			</ShellContext.Provider>
		</ThemeProvider>
	);
}
