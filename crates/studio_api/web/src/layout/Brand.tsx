import { Bolt, ChevronsUpDown, PanelLeft } from "lucide-react";
import { useSidebar } from "@/components/ui/sidebar";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuLabel,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { BrandConfig, SidebarConfig } from "@/lib/studio-config";

/// Resolves the logo source. Treats short strings as glyph (emoji or
/// single grapheme) and longer strings as URL/path/data-URL.
function isGlyph(s: string): boolean {
	// Glyphs are typically 1-4 chars and don't contain `/`, `.`, `:`.
	return s.length <= 4 && !/[\/.:]/.test(s);
}

export function Brand({
	brand,
	sidebar,
	manifestName,
	manifestVersion,
}: {
	brand: BrandConfig | undefined;
	sidebar: SidebarConfig | undefined;
	manifestName: string;
	manifestVersion: string;
}) {
	const { toggleSidebar } = useSidebar();
	const name = brand?.name ?? manifestName;
	const subtitle = brand?.subtitle ?? `v${manifestVersion}`;
	const logo = brand?.logo;
	const orgs = sidebar?.orgSwitcher?.items;
	const showCollapse = sidebar?.collapsible !== false;

	const Logo = (
		<div className="flex size-9 items-center justify-center rounded-md bg-primary text-primary-foreground shrink-0">
			{logo ? (
				isGlyph(logo) ? (
					<span className="text-base font-semibold leading-none">{logo}</span>
				) : (
					<img src={logo} alt="" className="size-5 object-contain" />
				)
			) : (
				<Bolt className="size-4" />
			)}
		</div>
	);

	return (
		<div className="flex items-center gap-2 px-2 py-2">
			{orgs && orgs.length > 0 ? (
				<DropdownMenu>
					<DropdownMenuTrigger asChild>
						<button
							type="button"
							className="flex flex-1 items-center gap-2 rounded-md px-1.5 py-1 hover:bg-sidebar-accent text-left"
						>
							{Logo}
							<div className="flex flex-col leading-tight min-w-0 flex-1">
								<span className="text-sm font-semibold truncate">{name}</span>
								<span className="text-xs text-muted-foreground truncate">
									{subtitle}
								</span>
							</div>
							<ChevronsUpDown className="size-3.5 text-muted-foreground" />
						</button>
					</DropdownMenuTrigger>
					<DropdownMenuContent align="start" className="min-w-56">
						<DropdownMenuLabel className="text-xs text-muted-foreground">
							Organizations
						</DropdownMenuLabel>
						<DropdownMenuSeparator />
						{orgs.map((o) => (
							<DropdownMenuItem key={o.id}>{o.label}</DropdownMenuItem>
						))}
					</DropdownMenuContent>
				</DropdownMenu>
			) : (
				<div className="flex flex-1 items-center gap-2 px-1.5 py-1 min-w-0">
					{Logo}
					<div className="flex flex-col leading-tight min-w-0">
						<span className="text-sm font-semibold truncate">{name}</span>
						<span className="text-xs text-muted-foreground truncate">
							{subtitle}
						</span>
					</div>
				</div>
			)}
			{showCollapse && (
				<button
					type="button"
					onClick={toggleSidebar}
					title="Collapse sidebar"
					className="rounded-md p-1.5 text-muted-foreground hover:bg-sidebar-accent hover:text-foreground"
				>
					<PanelLeft className="size-4" />
				</button>
			)}
		</div>
	);
}
