// Icon name → lucide component lookup.
//
// Users reference icons in `studio.config.ts` by string name (kebab or
// camel — we accept both, normalized to kebab here). Anything not in
// this allow-list falls back to the default. The allow-list is curated
// — no `import * as Icons from "lucide-react"` because that drags every
// icon into the bundle (~2 MB).

import {
	Activity,
	BadgeCheck,
	BarChart3,
	Bolt,
	Box,
	Boxes,
	Calendar,
	Database,
	FileCode,
	FileText,
	Files,
	Folder,
	Globe,
	Grid2x2,
	Inbox,
	Key,
	Layers,
	LayoutDashboard,
	LifeBuoy,
	Lock,
	Mail,
	MessagesSquare,
	Package,
	Receipt,
	Radio,
	Settings,
	Shield,
	ShieldCheck,
	ShoppingCart,
	Star,
	Tag,
	Terminal,
	Users,
	UserCheck,
	UserCog,
	Wand2,
	Zap,
} from "lucide-react";

type IconComponent = React.ComponentType<{ className?: string }>;

const TABLE: Record<string, IconComponent> = {
	activity: Activity,
	"badge-check": BadgeCheck,
	"bar-chart": BarChart3,
	bolt: Bolt,
	box: Box,
	boxes: Boxes,
	calendar: Calendar,
	database: Database,
	"file-code": FileCode,
	"file-text": FileText,
	files: Files,
	folder: Folder,
	globe: Globe,
	grid: Grid2x2,
	inbox: Inbox,
	key: Key,
	layers: Layers,
	"layout-dashboard": LayoutDashboard,
	"life-buoy": LifeBuoy,
	lock: Lock,
	mail: Mail,
	"messages-square": MessagesSquare,
	package: Package,
	receipt: Receipt,
	radio: Radio,
	settings: Settings,
	shield: Shield,
	"shield-check": ShieldCheck,
	"shopping-cart": ShoppingCart,
	star: Star,
	tag: Tag,
	terminal: Terminal,
	users: Users,
	"user-check": UserCheck,
	"user-cog": UserCog,
	"wand-2": Wand2,
	zap: Zap,
};

function camelToKebab(s: string): string {
	return s
		.replace(/([a-z0-9])([A-Z])/g, "$1-$2")
		.replace(/_/g, "-")
		.toLowerCase();
}

export function resolveIcon(name: string | undefined, fallback: IconComponent = Layers): IconComponent {
	if (!name) return fallback;
	const key = camelToKebab(name);
	return TABLE[key] ?? fallback;
}
