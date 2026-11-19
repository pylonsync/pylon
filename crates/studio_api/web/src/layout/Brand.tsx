import { Bolt, ChevronsUpDown } from "lucide-react";
import {
	Menu,
	MenuContent,
	MenuGroup,
	MenuGroupLabel,
	MenuItem,
	MenuTrigger,
} from "@/components/ui/menu";
import { onLinkClick } from "@/lib/navigation";
import { STUDIO_BASE } from "@/lib/route";
import type { BrandConfig, SidebarConfig } from "@/lib/studio-config";

/** A short string without path characters is a glyph. Anything else is an image URL. */
function isGlyph(s: string): boolean {
	return s.length <= 4 && !/[/.:]/.test(s);
}

function Logo({ logo }: { logo: string | undefined }) {
	return (
		<span className="flex size-5 shrink-0 items-center justify-center rounded bg-primary text-primary-foreground">
			{logo ? (
				isGlyph(logo) ? (
					<span className="text-[11px] leading-none font-semibold">{logo}</span>
				) : (
					<img src={logo} alt="" className="size-3.5 object-contain" />
				)
			) : (
				<Bolt className="size-3" />
			)}
		</span>
	);
}

/**
 * The app name and version at the top of the sidebar. It links to the Studio
 * home page, unless the config gives it an organization menu.
 */
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
	const name = brand?.name ?? manifestName;
	const subtitle = brand?.subtitle ?? `v${manifestVersion}`;
	const orgs = sidebar?.orgSwitcher?.items;

	const identity = (
		<>
			<Logo logo={brand?.logo} />
			<span className="min-w-0 flex-1 truncate text-[13px] font-semibold">
				{name}
			</span>
			<span className="shrink-0 truncate text-[11px] text-muted-foreground tabular-nums">
				{subtitle}
			</span>
		</>
	);

	if (orgs && orgs.length > 0) {
		return (
			<Menu>
				<MenuTrigger className="flex h-7 w-full cursor-default items-center gap-2 rounded-md px-1.5 text-left hover:bg-accent">
					{identity}
					<ChevronsUpDown className="size-3 shrink-0 text-muted-foreground" />
				</MenuTrigger>
				<MenuContent align="start" className="min-w-48">
					<MenuGroup>
						<MenuGroupLabel>Organizations</MenuGroupLabel>
						{orgs.map((o) => (
							<MenuItem key={o.id} value={o.id}>
								{o.label}
							</MenuItem>
						))}
					</MenuGroup>
				</MenuContent>
			</Menu>
		);
	}

	return (
		<a
			href={STUDIO_BASE}
			onClick={(e) => onLinkClick(e, STUDIO_BASE)}
			className="flex h-7 items-center gap-2 rounded-md px-1.5 text-foreground no-underline hover:bg-accent focus-visible:outline-2 focus-visible:outline-ring"
		>
			{identity}
		</a>
	);
}
