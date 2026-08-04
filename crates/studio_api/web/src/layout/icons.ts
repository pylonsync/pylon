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
	Ban,
	BarChart3,
	Bell,
	Bolt,
	Box,
	Boxes,
	Calendar,
	Check,
	Clock,
	Copy,
	CreditCard,
	Database,
	Download,
	ExternalLink,
	Eye,
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
	Link as LinkIcon,
	Lock,
	Mail,
	MessagesSquare,
	Package,
	Pencil,
	Play,
	Receipt,
	Radio,
	RefreshCw,
	Send,
	Settings,
	Share2,
	Shield,
	ShieldCheck,
	ShoppingCart,
	Star,
	Tag,
	Terminal,
	Trash2,
	Upload,
	Users,
	UserCheck,
	UserCog,
	Wand2,
	X,
	Zap,
} from "lucide-react";

type IconComponent = React.ComponentType<{ className?: string }>;

const TABLE: Record<string, IconComponent> = {
	activity: Activity,
	"badge-check": BadgeCheck,
	ban: Ban,
	"bar-chart": BarChart3,
	bell: Bell,
	bolt: Bolt,
	box: Box,
	boxes: Boxes,
	calendar: Calendar,
	check: Check,
	clock: Clock,
	copy: Copy,
	"credit-card": CreditCard,
	database: Database,
	download: Download,
	"external-link": ExternalLink,
	eye: Eye,
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
	link: LinkIcon,
	lock: Lock,
	mail: Mail,
	"messages-square": MessagesSquare,
	package: Package,
	pencil: Pencil,
	play: Play,
	receipt: Receipt,
	radio: Radio,
	refresh: RefreshCw,
	"refresh-cw": RefreshCw,
	send: Send,
	settings: Settings,
	share: Share2,
	shield: Shield,
	"shield-check": ShieldCheck,
	"shopping-cart": ShoppingCart,
	star: Star,
	tag: Tag,
	terminal: Terminal,
	trash: Trash2,
	"trash-2": Trash2,
	upload: Upload,
	users: Users,
	"user-check": UserCheck,
	"user-cog": UserCog,
	"wand-2": Wand2,
	x: X,
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
