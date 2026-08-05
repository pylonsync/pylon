import { ChevronRight } from "lucide-react";

export interface BreadcrumbCrumb {
	label: string;
	icon?: React.ComponentType<{ className?: string }>;
}

export function Breadcrumbs({ crumbs }: { crumbs: BreadcrumbCrumb[] }) {
	return (
		<nav
			className="flex min-w-0 items-center gap-1.5 text-sm"
			aria-label="Breadcrumb"
		>
			{crumbs.map((c, i) => {
				const last = i === crumbs.length - 1;
				const Icon = c.icon;
				return (
					// Only the trailing crumb may shrink, and it truncates rather
					// than pushing the header's badge off a phone screen. The
					// leading crumbs stay whole — a clipped "Dash…" reads as a bug.
					<span
						key={i}
						className={
							last
								? "flex min-w-0 items-center gap-1.5"
								: "flex shrink-0 items-center gap-1.5"
						}
					>
						{i > 0 && (
							<ChevronRight className="size-3.5 shrink-0 text-muted-foreground" />
						)}
						<span
							className={
								last
									? "flex min-w-0 items-center gap-1.5 font-medium text-foreground"
									: "flex items-center gap-1.5 text-muted-foreground"
							}
						>
							{Icon && <Icon className="size-3.5 shrink-0" />}
							<span className="truncate">{c.label}</span>
						</span>
					</span>
				);
			})}
		</nav>
	);
}
