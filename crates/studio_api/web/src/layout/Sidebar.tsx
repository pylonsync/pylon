import { createElement } from "react";
import { ChevronsUpDown, ExternalLink, Lock, LogOut, Settings } from "lucide-react";

import {
	Menu,
	MenuContent,
	MenuGroup,
	MenuGroupLabel,
	MenuItem,
	MenuRadioGroup,
	MenuRadioItem,
	MenuSeparator,
	MenuTrigger,
} from "@/components/ui/menu";
import { usePreference } from "@ui-sdk/react/preferences";
import { displayName, useAuth } from "@/auth/auth";
import { Brand } from "@/layout/Brand";
import { Icon } from "@/layout/icon";
import {
	resolveFooter,
	type ResolvedNavItem,
	type ResolvedNavSection,
} from "@/layout/resolve-nav";
import type { StudioRoute } from "@/layout/StudioLayout";
import { useCounts } from "@/lib/counts";
import { getExtensionLayoutSlot } from "@/lib/extensions";
import { onLinkClick, pushUrl } from "@/lib/navigation";
import { MANIFEST } from "@/lib/pylon";
import { routeToPath } from "@/lib/route";
import type { SidebarFooter, StudioConfig } from "@/lib/studio-config";
import { COLOR_SCHEMES, colorScheme, type ColorSchemeChoice } from "@/lib/theme";
import { cn } from "@/lib/utils";

const ITEM =
	"group/item flex h-7 w-full cursor-default items-center gap-2 rounded-md px-2 text-[13px] max-md:h-9 text-muted-foreground outline-offset-[-2px] hover:bg-accent/60 hover:text-foreground aria-[current=page]:bg-accent aria-[current=page]:font-medium aria-[current=page]:text-foreground [&>svg]:size-3.5 [&>svg]:shrink-0";

/**
 * The navigation: app name, nav groups, and the account menu.
 *
 * `onNavigate` runs after an item is chosen. The narrow layout uses it to
 * close the sheet that holds the sidebar.
 */
export function Sidebar({
	config,
	sections,
	route,
	onNavigate,
	className,
}: {
	config: StudioConfig;
	sections: ResolvedNavSection[];
	route: StudioRoute;
	onNavigate?: () => void;
	className?: string;
}) {
	const auth = useAuth();
	const isAdmin = !!auth.me?.is_admin;
	const counts = useCounts();
	const footer = resolveFooter(config.sidebar);

	return (
		<div className={cn("flex h-full min-h-0 flex-col bg-sidebar", className)}>
			<div className="flex h-11 shrink-0 items-center border-b px-1.5">
				<div className="min-w-0 flex-1">
					<Brand
						brand={config.brand}
						sidebar={config.sidebar}
						manifestName={MANIFEST.name}
						manifestVersion={MANIFEST.version}
					/>
				</div>
			</div>

			<nav aria-label="Studio" className="min-h-0 flex-1 overflow-y-auto px-1.5 py-2">
				{sections.map((section, i) => (
					<div
						key={`${section.label}-${i}`}
						role="group"
						aria-label={section.label || undefined}
						className="mb-3 last:mb-0"
					>
						{section.label && (
							<div className="flex h-6 items-center px-2 text-[11px] font-medium text-subtle-foreground">
								{section.label}
							</div>
						)}
						<ul>
							{section.items.map((item, j) => (
								<li key={`${item.kind}-${j}`}>
									<NavItem
										item={item}
										route={route}
										isAdmin={isAdmin}
										count={
											item.kind === "resource" ? counts[item.entity] : undefined
										}
										onNavigate={onNavigate}
									/>
								</li>
							))}
						</ul>
					</div>
				))}
			</nav>

			<FooterSlot footer={footer} />

			<div className="shrink-0 border-t p-1.5">
				<AccountMenu account={displayName(auth)} onNavigate={onNavigate} />
			</div>
		</div>
	);
}

function NavItem({
	item,
	route,
	isAdmin,
	count,
	onNavigate,
}: {
	item: ResolvedNavItem;
	route: StudioRoute;
	isAdmin: boolean;
	count: number | undefined;
	onNavigate?: () => void;
}) {
	if (item.kind === "heading") {
		return (
			<div className="flex h-6 items-center px-2 pt-1 text-[11px] font-medium text-subtle-foreground">
				{item.label}
			</div>
		);
	}

	if (item.kind === "link") {
		return (
			<a
				href={item.href}
				target={item.external ? "_blank" : undefined}
				rel={item.external ? "noreferrer" : undefined}
				className={ITEM}
				onClick={onNavigate}
			>
				<Icon name={item.icon} fallback={ExternalLink} />
				<span className="min-w-0 flex-1 truncate">{item.label}</span>
				{item.external && <ExternalLink className="size-3! opacity-60" />}
			</a>
		);
	}

	const target: StudioRoute =
		item.kind === "page"
			? { kind: "page", id: item.id }
			: { kind: "resource", entity: item.entity };
	const current =
		(target.kind === "page" && route.kind === "page" && route.id === target.id) ||
		(target.kind === "resource" &&
			route.kind === "resource" &&
			route.entity === target.entity);
	const locked = item.requiresAdmin && !isAdmin;
	const href = routeToPath(target);

	return (
		<a
			href={href}
			aria-current={current ? "page" : undefined}
			title={locked ? `${item.label}: an admin account is necessary` : undefined}
			className={ITEM}
			onClick={(e) => {
				onLinkClick(e, href);
				if (e.defaultPrevented) onNavigate?.();
			}}
		>
			<Icon name={item.icon} />
			<span className="min-w-0 flex-1 truncate">{item.label}</span>
			{locked ? (
				<Lock className="size-3! opacity-60" />
			) : (
				count !== undefined && (
					<span className="shrink-0 text-[11px] text-subtle-foreground tabular-nums group-aria-[current=page]/item:text-muted-foreground">
						{count.toLocaleString()}
					</span>
				)
			)}
		</a>
	);
}

/** The optional footer from studio.config.ts: a card, or an extension component. */
function FooterSlot({ footer }: { footer: SidebarFooter | null }) {
	if (!footer) return null;
	if (footer.type === "custom") {
		const custom = getExtensionLayoutSlot("footer");
		return custom ? (
			<div className="shrink-0 border-t p-1.5">{createElement(custom)}</div>
		) : null;
	}
	return (
		<div className="shrink-0 border-t px-3.5 py-2.5">
			<div className="text-xs font-medium">{footer.title}</div>
			<p className="mt-0.5 text-xs text-muted-foreground">{footer.description}</p>
			{typeof footer.progress === "number" && (
				<div className="mt-2 h-1 w-full overflow-hidden rounded-full bg-border">
					<div
						className="h-full rounded-full bg-primary"
						style={{
							width: `${Math.min(100, Math.max(0, footer.progress * 100))}%`,
						}}
					/>
				</div>
			)}
			{footer.action && (
				<a
					href={footer.action.href}
					target={/^https?:\/\//.test(footer.action.href) ? "_blank" : undefined}
					rel="noreferrer"
					className="mt-1.5 inline-block text-xs font-medium text-link hover:underline"
				>
					{footer.action.label}
				</a>
			)}
		</div>
	);
}

const SCHEME_LABEL: Record<ColorSchemeChoice, string> = {
	system: "System",
	light: "Light",
	dark: "Dark",
};

/** The signed-in operator, with the menu for settings, theme and sign out. */
function AccountMenu({
	account,
	onNavigate,
}: {
	account: string;
	onNavigate?: () => void;
}) {
	const { value: scheme } = usePreference(colorScheme);
	return (
		<Menu>
			<MenuTrigger
				aria-label={`Account: ${account}`}
				className="flex h-8 w-full cursor-default items-center gap-2 rounded-md px-1.5 text-left hover:bg-accent data-open:bg-accent"
			>
				<span className="flex size-5 shrink-0 items-center justify-center rounded-full bg-accent text-[10px] font-semibold text-foreground ring-1 ring-border-strong">
					{account.slice(0, 1).toUpperCase()}
				</span>
				<span className="min-w-0 flex-1 truncate text-[13px]">{account}</span>
				<ChevronsUpDown className="size-3 shrink-0 text-muted-foreground" />
			</MenuTrigger>
			<MenuContent side="top" align="start" className="min-w-(--anchor-width)">
				<MenuItem
					onSelect={() => {
						pushUrl(routeToPath({ kind: "page", id: "settings" }));
						onNavigate?.();
					}}
				>
					<Settings />
					Settings
				</MenuItem>
				<MenuSeparator />
				<MenuGroup>
					<MenuGroupLabel>Theme</MenuGroupLabel>
					<MenuRadioGroup
						value={scheme}
						onValueChange={(next) => colorScheme.set(next as ColorSchemeChoice)}
					>
						{COLOR_SCHEMES.map((s) => (
							<MenuRadioItem key={s} value={s} closeOnSelect={false}>
								{SCHEME_LABEL[s]}
							</MenuRadioItem>
						))}
					</MenuRadioGroup>
				</MenuGroup>
				<MenuSeparator />
				{/* A full page load: /studio/logout ends the session on the server
				    and redirects to the sign-in page. */}
				<MenuItem render={<a href="/studio/logout" />}>
					<LogOut />
					Sign out
				</MenuItem>
			</MenuContent>
		</Menu>
	);
}
