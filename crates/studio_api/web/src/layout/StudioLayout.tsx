import { useMemo } from "react";
import { ExternalLink, LayoutDashboard, Lock, LogOut, Settings } from "lucide-react";
import {
	Sidebar,
	SidebarContent,
	SidebarFooter,
	SidebarGroup,
	SidebarGroupContent,
	SidebarGroupLabel,
	SidebarHeader,
	SidebarInset,
	SidebarMenu,
	SidebarMenuButton,
	SidebarMenuItem,
	SidebarProvider,
	SidebarTrigger,
	useSidebar,
} from "@/components/ui/sidebar";
import { Badge } from "@/components/ui/badge";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuLabel,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { displayName, useAuth } from "@/auth/AuthContext";
import { LockedPage } from "@/pages/Locked";
import { MANIFEST } from "@/lib/pylon";
import type { StudioConfig } from "@/lib/studio-config";
import { Brand } from "@/layout/Brand";
import { Breadcrumbs, type BreadcrumbCrumb } from "@/layout/Breadcrumbs";
import { SidebarFooterCard } from "@/layout/SidebarFooterCard";
import { ThemeProvider } from "@/layout/ThemeProvider";
import { resolveIcon } from "@/layout/icons";
import {
	defaultFooter,
	resolveNav,
	type ResolvedNavItem,
} from "@/layout/resolve-nav";

/// Identifier for what the user is currently viewing. Discriminated by
/// `kind` so the App can render the right page for each.
export type StudioRoute =
	| { kind: "page"; id: string }
	| { kind: "resource"; entity: string };

export function routeKey(r: StudioRoute): string {
	return r.kind === "page" ? `page:${r.id}` : `resource:${r.entity}`;
}

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
	const { me, user, loading } = useAuth();
	const isAdmin = !!me?.is_admin;
	const account = displayName(user, me);

	const sections = useMemo(() => resolveNav(config, MANIFEST), [config]);
	const footer = useMemo(() => defaultFooter(config.sidebar), [config.sidebar]);

	// The server refuses to serve this bundle to anyone but a signed-in admin,
	// so reaching an un-admin state here means the session ended while the tab
	// was open. Say so and offer the way back rather than leaving every panel
	// to fail its own fetch with a 401.
	const sessionLost = !loading && !isAdmin;

	const crumbs = useMemo<BreadcrumbCrumb[]>(() => {
		const head: BreadcrumbCrumb = {
			label: "Dashboard",
			icon: LayoutDashboard,
		};
		const current = locateCurrent(route, sections);
		if (!current) return [head];
		const Icon = current.icon ? resolveIcon(current.icon) : undefined;
		return [head, { label: current.label, icon: Icon }];
	}, [route, sections]);

	return (
		<ThemeProvider theme={config.theme}>
			<SidebarProvider>
				<Sidebar variant="inset">
					<SidebarHeader>
						<Brand
							brand={config.brand}
							sidebar={config.sidebar}
							manifestName={MANIFEST.name}
							manifestVersion={MANIFEST.version}
						/>
					</SidebarHeader>
					<SidebarContent>
						{sections.map((section, idx) => (
							<SidebarGroup key={`${section.label}-${idx}`}>
								{section.label && (
									<SidebarGroupLabel className="text-[11px] tracking-[0.08em] uppercase">
										{section.label}
									</SidebarGroupLabel>
								)}
								<SidebarGroupContent>
									<SidebarMenu>
										{section.items.map((item, j) => (
											<NavItem
												key={`${item.kind}-${j}`}
												item={item}
												route={route}
												onRouteChange={onRouteChange}
												isAdmin={isAdmin}
											/>
										))}
									</SidebarMenu>
								</SidebarGroupContent>
							</SidebarGroup>
						))}
					</SidebarContent>
					<SidebarFooter className="gap-0 p-0">
						{footer && footer.type === "card" && (
							<SidebarFooterCard footer={footer} />
						)}
						<AccountMenu
							account={account}
							isAdmin={isAdmin}
							onRouteChange={onRouteChange}
						/>
					</SidebarFooter>
				</Sidebar>
				<SidebarInset className="min-w-0">
					<header className="flex h-14 shrink-0 items-center gap-2 border-b px-4 sm:gap-3 sm:px-6">
						{/* Below `md` the sidebar is a closed sheet, and the only other
						    toggle lives inside it (Brand's collapse button) — so without
						    this the nav is unreachable on a phone. `md:hidden` matches
						    the 768px breakpoint `useIsMobile` switches on. */}
						<SidebarTrigger className="-ml-1 size-8 md:hidden" />
						<Breadcrumbs crumbs={crumbs} />
						<div className="ml-auto flex shrink-0 items-center gap-2">
							{!loading && (
								<Badge variant={isAdmin ? "default" : "destructive"}>
									{isAdmin ? "Admin" : "Session ended"}
								</Badge>
							)}
						</div>
					</header>
					<div className="min-w-0 px-4 py-6 sm:px-6">
						{sessionLost ? (
							<LockedPage
								title="Your session ended"
								description="Studio follows your account on this app. Sign in again as an admin to continue."
								action={{ label: "Sign in", href: "/studio/logout" }}
							/>
						) : (
							children
						)}
					</div>
				</SidebarInset>
			</SidebarProvider>
		</ThemeProvider>
	);
}

/// The signed-in account row at the foot of the nav, with its menu.
///
/// Its own component for the same reason as [`NavItem`]: `useSidebar()` is
/// only reachable below `SidebarProvider`, and "Settings" has to dismiss the
/// mobile sheet or it navigates behind an open menu.
function AccountMenu({
	account,
	isAdmin,
	onRouteChange,
}: {
	account: string;
	isAdmin: boolean;
	onRouteChange: (r: StudioRoute) => void;
}) {
	const { isMobile, setOpenMobile } = useSidebar();
	return (
		<SidebarMenu className="px-2 pb-2">
			<SidebarMenuItem>
				<DropdownMenu>
					<DropdownMenuTrigger asChild>
						<SidebarMenuButton tooltip="Account">
							<div className="flex size-6 items-center justify-center rounded-full bg-primary text-xs font-semibold text-primary-foreground">
								{account.slice(0, 1).toUpperCase()}
							</div>
							<div className="flex min-w-0 flex-col items-start leading-tight">
								<span className="max-w-[10rem] truncate text-xs font-medium">
									{account}
								</span>
								<span className="text-[10px] text-muted-foreground">
									{isAdmin ? "Admin" : "Session ended"}
								</span>
							</div>
						</SidebarMenuButton>
					</DropdownMenuTrigger>
					{/* `side="right"` would open off-screen inside a 288px mobile
					    sheet; above the trigger is the only direction with room. */}
					<DropdownMenuContent
						side={isMobile ? "top" : "right"}
						align="end"
						className="min-w-44"
					>
						<DropdownMenuLabel className="text-xs font-normal">
							{account}
						</DropdownMenuLabel>
						<DropdownMenuSeparator />
						<DropdownMenuItem
							onClick={() => {
								onRouteChange({ kind: "page", id: "settings" });
								if (isMobile) setOpenMobile(false);
							}}
						>
							<Settings className="size-4" />
							Settings
						</DropdownMenuItem>
						{/* A full navigation, not a fetch: /studio/logout revokes the
						    session server-side and redirects to the app's login page,
						    so there's nothing left for this tab to render. */}
						<DropdownMenuItem asChild>
							<a href="/studio/logout">
								<LogOut className="size-4" />
								Sign out
							</a>
						</DropdownMenuItem>
					</DropdownMenuContent>
				</DropdownMenu>
			</SidebarMenuItem>
		</SidebarMenu>
	);
}

/// One sidebar entry.
///
/// A component rather than a render function so it can reach `useSidebar()`:
/// on mobile the nav is a sheet overlaying the page, and tapping an item has
/// to dismiss it. Without that the route changes behind a menu that stays
/// open on top of the thing you just navigated to.
function NavItem({
	item,
	route,
	onRouteChange,
	isAdmin,
}: {
	item: ResolvedNavItem;
	route: StudioRoute;
	onRouteChange: (r: StudioRoute) => void;
	isAdmin: boolean;
}): React.ReactNode {
	const { isMobile, setOpenMobile } = useSidebar();
	const dismissOnMobile = () => {
		if (isMobile) setOpenMobile(false);
	};

	if (item.kind === "heading") {
		return (
			<div className="px-2 pt-3 pb-1 text-[10px] uppercase tracking-wider text-muted-foreground/80">
				{item.label}
			</div>
		);
	}

	if (item.kind === "link") {
		const Icon = resolveIcon(item.icon, ExternalLink);
		return (
			<SidebarMenuItem>
				<SidebarMenuButton asChild tooltip={item.label}>
					<a
						href={item.href}
						target={item.external ? "_blank" : undefined}
						rel={item.external ? "noreferrer" : undefined}
						// An external link opens a new tab and leaves the sheet
						// covering this one; a same-tab link navigates away and the
						// sheet would flash over the new page. Close either way.
						onClick={dismissOnMobile}
					>
						<Icon />
						<span>{item.label}</span>
						{item.external && (
							<ExternalLink className="ml-auto size-3 opacity-60" />
						)}
					</a>
				</SidebarMenuButton>
			</SidebarMenuItem>
		);
	}

	const locked = item.requiresAdmin && !isAdmin;
	const isActive =
		(item.kind === "page" && route.kind === "page" && route.id === item.id) ||
		(item.kind === "resource" &&
			route.kind === "resource" &&
			route.entity === item.entity);
	const Icon = resolveIcon(item.icon);
	const tooltip = locked ? `${item.label} — admin required` : item.label;

	return (
		<SidebarMenuItem>
			<SidebarMenuButton
				isActive={isActive}
				tooltip={tooltip}
				onClick={() => {
					if (item.kind === "page") onRouteChange({ kind: "page", id: item.id });
					else onRouteChange({ kind: "resource", entity: item.entity });
					dismissOnMobile();
				}}
			>
				<Icon />
				<span>{item.label}</span>
				{locked && <Lock className="ml-auto size-3 opacity-60" />}
			</SidebarMenuButton>
		</SidebarMenuItem>
	);
}

function locateCurrent(
	route: StudioRoute,
	sections: ReturnType<typeof resolveNav>,
): { label: string; icon?: string } | null {
	for (const s of sections) {
		for (const item of s.items) {
			if (item.kind === "page" && route.kind === "page" && item.id === route.id) {
				return { label: item.label, icon: item.icon };
			}
			if (
				item.kind === "resource" &&
				route.kind === "resource" &&
				item.entity === route.entity
			) {
				return { label: item.label, icon: item.icon };
			}
		}
	}
	if (route.kind === "resource") return { label: route.entity };
	if (route.kind === "page") return { label: capitalize(route.id) };
	return null;
}

function capitalize(s: string): string {
	if (!s) return "";
	return s.charAt(0).toUpperCase() + s.slice(1);
}
