import { useMemo, useState } from "react";
import { ExternalLink, LayoutDashboard, Lock, LogIn, LogOut, Settings } from "lucide-react";
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
} from "@/components/ui/sidebar";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuLabel,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useAuth } from "@/auth/AuthContext";
import { SignInDialog } from "@/auth/SignInDialog";
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
	const { me, hasToken, loading, signOut } = useAuth();
	const [signInOpen, setSignInOpen] = useState(false);
	const isAdmin = !!me?.is_admin;
	// Cookie-authed users don't have a Bearer token in localStorage but
	// /api/auth/me resolves their session and sets `me.user_id`. Treat
	// "has a resolved session" as authed. hasToken is OR'd in for the
	// Bearer path so legacy admin-token signins still register.
	const isAuthed = !!me?.user_id || hasToken;

	const sections = useMemo(() => resolveNav(config, MANIFEST), [config]);
	const footer = useMemo(() => defaultFooter(config.sidebar), [config.sidebar]);

	// Block the main content area for unauthenticated callers.
	const requireAuth = !loading && !isAuthed;

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
										{section.items.map((item, j) =>
											renderItem(item, j, route, onRouteChange, isAdmin),
										)}
									</SidebarMenu>
								</SidebarGroupContent>
							</SidebarGroup>
						))}
					</SidebarContent>
					<SidebarFooter className="gap-0 p-0">
						{footer && footer.type === "card" && (
							<SidebarFooterCard footer={footer} />
						)}
						<SidebarMenu className="px-2 pb-2">
							<SidebarMenuItem>
								{isAuthed ? (
									<DropdownMenu>
										<DropdownMenuTrigger asChild>
											<SidebarMenuButton tooltip="Account">
												<div className="flex size-6 items-center justify-center rounded-full bg-primary text-xs font-semibold text-primary-foreground">
													{isAdmin
														? "A"
														: me?.user_id?.slice(0, 1).toUpperCase() ?? "U"}
												</div>
												<div className="flex flex-col items-start leading-tight">
													<span className="text-xs font-medium">
														{isAdmin ? "Admin" : me?.user_id ?? "Signed in"}
													</span>
													<span className="text-[10px] text-muted-foreground">
														{isAdmin ? "Full access" : "Limited"}
													</span>
												</div>
											</SidebarMenuButton>
										</DropdownMenuTrigger>
										<DropdownMenuContent
											side="right"
											align="end"
											className="min-w-44"
										>
											<DropdownMenuLabel className="text-xs font-normal">
												{me?.user_id ?? "anonymous"}
											</DropdownMenuLabel>
											<DropdownMenuSeparator />
											<DropdownMenuItem
												onClick={() =>
													onRouteChange({ kind: "page", id: "settings" })
												}
											>
												<Settings className="size-4" />
												Settings
											</DropdownMenuItem>
											<DropdownMenuItem onClick={signOut}>
												<LogOut className="size-4" />
												Sign out
											</DropdownMenuItem>
										</DropdownMenuContent>
									</DropdownMenu>
								) : (
									<SidebarMenuButton onClick={() => setSignInOpen(true)}>
										<LogIn />
										<span>Sign in</span>
									</SidebarMenuButton>
								)}
							</SidebarMenuItem>
						</SidebarMenu>
					</SidebarFooter>
				</Sidebar>
				<SidebarInset>
					<header className="flex h-14 shrink-0 items-center gap-3 border-b px-6">
						<Breadcrumbs crumbs={crumbs} />
						<div className="ml-auto flex items-center gap-2">
							{!isAuthed && (
								<Button
									size="sm"
									variant="outline"
									onClick={() => setSignInOpen(true)}
								>
									<LogIn className="size-3.5" /> Sign in
								</Button>
							)}
							{isAuthed && (
								<Badge variant={isAdmin ? "default" : "secondary"}>
									{isAdmin ? "Admin" : "Signed in"}
								</Badge>
							)}
						</div>
					</header>
					<div className="px-6 py-6">
						{requireAuth ? (
							<LockedPage
								title="Sign in to Pylon Studio"
								description="Studio surfaces your live data, schema, and operations. Sign in with PYLON_ADMIN_TOKEN (or your user token) to continue."
							/>
						) : (
							children
						)}
					</div>
				</SidebarInset>
				<SignInDialog open={signInOpen} onOpenChange={setSignInOpen} />
			</SidebarProvider>
		</ThemeProvider>
	);
}

function renderItem(
	item: ResolvedNavItem,
	key: number,
	route: StudioRoute,
	onRouteChange: (r: StudioRoute) => void,
	isAdmin: boolean,
): React.ReactNode {
	if (item.kind === "heading") {
		return (
			<div
				key={`h-${key}`}
				className="px-2 pt-3 pb-1 text-[10px] uppercase tracking-wider text-muted-foreground/80"
			>
				{item.label}
			</div>
		);
	}

	if (item.kind === "link") {
		const Icon = resolveIcon(item.icon, ExternalLink);
		return (
			<SidebarMenuItem key={`l-${key}`}>
				<SidebarMenuButton asChild tooltip={item.label}>
					<a
						href={item.href}
						target={item.external ? "_blank" : undefined}
						rel={item.external ? "noreferrer" : undefined}
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
		<SidebarMenuItem key={`i-${key}`}>
			<SidebarMenuButton
				isActive={isActive}
				tooltip={tooltip}
				onClick={() => {
					if (item.kind === "page") onRouteChange({ kind: "page", id: item.id });
					else
						onRouteChange({ kind: "resource", entity: item.entity });
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
