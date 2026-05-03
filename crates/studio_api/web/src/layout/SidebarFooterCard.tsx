import type { SidebarFooter } from "@/lib/studio-config";

/**
 * The bottom card in the screenshot — by default rendered as a
 * "Used space" hint with optional progress bar + CTA. Custom
 * (componentId) footers resolve via the extensions registry; the
 * built-in implementation here covers the card variant.
 */
export function SidebarFooterCard({ footer }: { footer: SidebarFooter | null }) {
	if (!footer) return null;
	if (footer.type !== "card") return null; // custom slot is handled by parent

	return (
		<div className="mx-2 mb-2 mt-3 rounded-lg border border-sidebar-border bg-sidebar-accent/40 p-3">
			<div className="text-sm font-semibold">{footer.title}</div>
			<p className="mt-1 text-xs text-muted-foreground leading-relaxed">
				{footer.description}
			</p>
			{typeof footer.progress === "number" && (
				<div className="mt-3 h-1 w-full overflow-hidden rounded-full bg-sidebar-border/60">
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
					className="mt-3 block text-xs font-medium text-primary hover:underline"
				>
					{footer.action.label}
				</a>
			)}
		</div>
	);
}
