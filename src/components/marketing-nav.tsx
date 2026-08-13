"use client";

import {
	type Dispatch,
	type SetStateAction,
	useEffect,
	useRef,
	useState,
} from "react";
import { Link } from "@pylonsync/react";
import { ChevronDown, Github, Menu, X } from "lucide-react";
import { PylonMark } from "./brand";
import { Button } from "./ui/button";
import { ThemeToggle } from "./theme-toggle";
import { cn } from "../lib/utils";
import { FRAME_COL } from "./marketing-frame";
import { COMPARISONS_ENABLED } from "../lib/comparison-content";
import {
	COMPARISONS,
	DEVELOPERS,
	PRODUCT_GROUPS,
	SOLUTIONS,
	type NavLink,
} from "../lib/site-nav";
import { accountUrl } from "../lib/account-urls";

// Marketing-site top nav with Supabase-style mega-menus. Shared across every
// public page so the structure (Product / Solutions / Developers / Compare /
// Pricing / Docs) is identical everywhere. Client component: the dropdowns +
// mobile sheet are interactive. The `signedIn` CTA is seeded from the SSR
// `auth` prop by each page so the right button is in the first paint.

type MenuKey = "product" | "solutions" | "developers" | "compare" | null;

function MenuLink({ link, onClick }: { link: NavLink; onClick?: () => void }) {
	const Icon = link.icon;
	const cls =
		"group/mi flex items-start gap-3 rounded-[var(--radius-md)] px-3 py-2.5 transition-colors hover:bg-[var(--color-paper-1)]";
	const body = (
		<>
			{Icon && (
				<span className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-[var(--radius-sm)] border border-[var(--color-rule)] bg-[var(--color-paper-1)] text-[var(--color-ink-2)] transition-colors group-hover/mi:border-[var(--color-brand)]/40 group-hover/mi:text-[var(--color-brand)]">
					<Icon className="size-[15px]" />
				</span>
			)}
			<span className="min-w-0">
				<span className="block text-[13.5px] font-medium tracking-tight text-[var(--color-ink)]">
					{link.label}
				</span>
				{link.desc && (
					<span className="mt-0.5 block text-[12.5px] leading-[1.45] text-[var(--color-ink-3)]">
						{link.desc}
					</span>
				)}
			</span>
		</>
	);
	return link.external ? (
		<a href={link.href} className={cls} onClick={onClick}>
			{body}
		</a>
	) : (
		<Link href={link.href} className={cls} onClick={onClick}>
			{body}
		</Link>
	);
}

// Top-level (not redefined per render) so React keeps the same component
// identity across MarketingNav re-renders — defining it inline would remount
// all four trigger buttons on every state change (losing focus + churning the
// DOM). `open`/`setOpen` are threaded in as props.
function NavTrigger({
	k,
	label,
	open,
	setOpen,
}: {
	k: NonNullable<MenuKey>;
	label: string;
	open: MenuKey;
	setOpen: Dispatch<SetStateAction<MenuKey>>;
}) {
	return (
		<button
			type="button"
			onClick={() => setOpen((cur) => (cur === k ? null : k))}
			onMouseEnter={() => open && setOpen(k)}
			aria-expanded={open === k}
			className={cn(
				"inline-flex items-center gap-1 rounded-[var(--radius-sm)] px-3 py-1.5 text-[13.5px] transition-colors",
				open === k
					? "bg-[var(--color-paper-1)] text-[var(--color-ink)]"
					: "text-[var(--color-ink-2)] hover:bg-[var(--color-paper-1)] hover:text-[var(--color-ink)]",
			)}
		>
			{label}
			<ChevronDown
				className={cn(
					"size-3.5 transition-transform",
					open === k && "rotate-180",
				)}
			/>
		</button>
	);
}

export function MarketingNav({ signedIn = false }: { signedIn?: boolean }) {
	const [open, setOpen] = useState<MenuKey>(null);
	const [mobileOpen, setMobileOpen] = useState(false);
	const navRef = useRef<HTMLDivElement>(null);

	// Close the open mega-menu on outside click / Escape.
	useEffect(() => {
		if (!open) return;
		function onDown(e: MouseEvent) {
			if (navRef.current && !navRef.current.contains(e.target as Node)) {
				setOpen(null);
			}
		}
		function onKey(e: KeyboardEvent) {
			if (e.key === "Escape") setOpen(null);
		}
		document.addEventListener("mousedown", onDown);
		document.addEventListener("keydown", onKey);
		return () => {
			document.removeEventListener("mousedown", onDown);
			document.removeEventListener("keydown", onKey);
		};
	}, [open]);

	const cta =
		signedIn === true ? (
			<Button asChild variant="primary" size="sm">
				<Link href={accountUrl("/dashboard")}>Dashboard →</Link>
			</Button>
		) : (
			<>
				<Button
					asChild
					variant="ghost"
					size="sm"
					className="hidden sm:inline-flex"
				>
					<Link href={accountUrl("/login")}>Sign in</Link>
				</Button>
				<Button asChild variant="primary" size="sm">
					<Link href={accountUrl("/signup")}>Create your account →</Link>
				</Button>
			</>
		);

	return (
		// Flush header band, aligned to the frame rails.
		//
		// This was a floating rounded pill that hovered over the page. Against
		// the framed layout it read as a separate object parked on top of the
		// drawing — a second set of rounded edges cutting across the two
		// hairlines that run the full height. A flush band inside the same
		// measure makes the header the drawing's top edge instead.
		//
		// The nav column carries FRAME_COL too, so the frame's two hairlines run
		// all the way to the top of the page instead of starting under the
		// header. No bottom rule: a full-width horizontal line here crossed both
		// verticals and read as a lid on the drawing.
		<div
			ref={navRef}
			className="sticky top-0 z-50 bg-[var(--color-paper)]/85 backdrop-blur-xl"
		>
			<nav
				className={`${FRAME_COL} flex h-[62px] items-center justify-between gap-3 px-5 sm:px-8`}
			>
				<div className="flex items-center gap-1">
					<Link
						href="/"
						className="mr-5 flex items-center gap-2 text-[var(--color-ink)]"
					>
						<PylonMark size={20} />
						<span className="text-[15px] font-semibold leading-none tracking-tight">
							Pylon
						</span>
					</Link>
					<div className="hidden items-center gap-0.5 lg:flex">
						<NavTrigger k="product" label="Product" open={open} setOpen={setOpen} />
						<NavTrigger k="solutions" label="Solutions" open={open} setOpen={setOpen} />
						<NavTrigger k="developers" label="Developers" open={open} setOpen={setOpen} />
						{COMPARISONS_ENABLED && (
							<NavTrigger k="compare" label="Compare" open={open} setOpen={setOpen} />
						)}
						<a
							href="https://docs.pylonsync.com"
							className="rounded-[var(--radius-sm)] px-3 py-1.5 text-[13.5px] text-[var(--color-ink-2)] transition-colors hover:bg-[var(--color-paper-1)] hover:text-[var(--color-ink)]"
						>
							Docs
						</a>
					</div>
				</div>

				<div className="flex items-center gap-2">
					<a
							href="https://github.com/pylonsync/pylon"
							target="_blank"
							rel="noreferrer"
							aria-label="Pylon on GitHub"
							className="hidden size-8 items-center justify-center rounded-[var(--radius-sm)] text-[var(--color-ink-3)] transition-colors hover:bg-[var(--color-paper-1)] hover:text-[var(--color-ink)] sm:inline-flex"
						>
							<Github className="size-[18px]" />
						</a>
						<ThemeToggle />
					{cta}
					<button
						type="button"
						aria-label={mobileOpen ? "Close menu" : "Open menu"}
						aria-expanded={mobileOpen}
						onClick={() => setMobileOpen((v) => !v)}
						className="inline-flex h-8 w-8 items-center justify-center rounded-[var(--radius-sm)] border border-[var(--color-rule)] bg-[var(--color-paper)] text-[var(--color-ink-2)] hover:bg-[var(--color-paper-1)] lg:hidden"
					>
						{mobileOpen ? <X className="size-4" /> : <Menu className="size-4" />}
					</button>
				</div>
			</nav>

			{/* Desktop mega-menu panel. Plain `block` (no `hidden lg:block`): the
			    triggers that set `open` live in the `hidden lg:flex` cluster, so
			    this only ever renders from a desktop click — no mobile guard
			    needed. (`hidden lg:block` mis-computed to display:none in the
			    compiled Tailwind here, while `hidden lg:flex` worked — so we
			    don't depend on `lg:block` winning over `hidden`.) */}
			{open && (
				<div className="absolute left-0 right-0 top-full block px-5 pt-2 sm:px-8">
					<div className="mx-auto max-w-[1280px] rounded-[var(--radius-xl)] border border-[var(--color-rule)] bg-[var(--color-paper)] px-5 py-6 shadow-[var(--shadow-dropdown)] sm:px-6">
						{open === "product" && (
							<div className="grid grid-cols-12 gap-6">
								<div className="col-span-9 grid grid-cols-2 gap-x-6 gap-y-1">
									{PRODUCT_GROUPS[0].links.map((l) => (
										<MenuLink key={l.href} link={l} onClick={() => setOpen(null)} />
									))}
								</div>
								<div className="col-span-3 border-l border-[var(--color-rule)] pl-6">
									<div className="mb-1 px-3 font-mono text-[10.5px] uppercase tracking-[0.1em] text-[var(--color-ink-4)]">
										Ship & clients
									</div>
									{[...PRODUCT_GROUPS[1].links, ...PRODUCT_GROUPS[2].links].map(
										(l) => (
											<MenuLink
												key={l.href}
												link={l}
												onClick={() => setOpen(null)}
											/>
										),
									)}
								</div>
							</div>
						)}
						{open === "solutions" && (
							<div className="grid grid-cols-2 gap-x-6 gap-y-1">
								{SOLUTIONS.map((l) => (
									<MenuLink key={l.href} link={l} onClick={() => setOpen(null)} />
								))}
							</div>
						)}
						{open === "developers" && (
							<div className="grid grid-cols-3 gap-x-6 gap-y-1">
								{DEVELOPERS.map((l) => (
									<MenuLink key={l.href} link={l} onClick={() => setOpen(null)} />
								))}
							</div>
						)}
						{COMPARISONS_ENABLED && open === "compare" && (
							<div className="grid max-w-[640px] grid-cols-2 gap-x-6 gap-y-1">
								{COMPARISONS.map((l) => (
									<MenuLink key={l.href} link={l} onClick={() => setOpen(null)} />
								))}
							</div>
						)}
					</div>
				</div>
			)}

			{/* Mobile sheet */}
			{mobileOpen && (
				<div className="absolute left-0 right-0 top-full px-5 pt-2 sm:px-8 lg:hidden">
					<div className="mx-auto flex max-h-[75vh] max-w-[1280px] flex-col gap-4 overflow-y-auto rounded-[var(--radius-xl)] border border-[var(--color-rule)] bg-[var(--color-paper)] px-5 py-5 shadow-[var(--shadow-dropdown)]">
						{[
							{ title: "Product", links: PRODUCT_GROUPS.flatMap((g) => g.links) },
							{ title: "Solutions", links: SOLUTIONS },
							{ title: "Developers", links: DEVELOPERS },
							...(COMPARISONS_ENABLED
								? [{ title: "Compare", links: COMPARISONS }]
								: []),
						].map((section) => (
							<div key={section.title}>
								<div className="mb-1 font-mono text-[10.5px] uppercase tracking-[0.1em] text-[var(--color-ink-4)]">
									{section.title}
								</div>
								<div className="flex flex-col">
									{section.links.map((l) => (
										<MenuLink
											key={l.href}
											link={l}
											onClick={() => setMobileOpen(false)}
										/>
									))}
								</div>
							</div>
						))}
						<div className="flex gap-2 border-t border-[var(--color-rule)] pt-4">
							<a
								href="https://docs.pylonsync.com"
								className="text-[14px] text-[var(--color-ink-2)]"
							>
								Docs
							</a>
						</div>
					</div>
				</div>
			)}
		</div>
	);
}
