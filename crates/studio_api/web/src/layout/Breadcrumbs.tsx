import { ChevronRight } from "lucide-react";

export interface BreadcrumbCrumb {
	label: string;
	icon?: React.ComponentType<{ className?: string }>;
}

export function Breadcrumbs({ crumbs }: { crumbs: BreadcrumbCrumb[] }) {
	return (
		<nav className="flex items-center gap-1.5 text-sm" aria-label="Breadcrumb">
			{crumbs.map((c, i) => {
				const last = i === crumbs.length - 1;
				const Icon = c.icon;
				return (
					<span key={i} className="flex items-center gap-1.5">
						{i > 0 && (
							<ChevronRight className="size-3.5 text-muted-foreground" />
						)}
						<span
							className={
								last
									? "flex items-center gap-1.5 font-medium text-foreground"
									: "flex items-center gap-1.5 text-muted-foreground"
							}
						>
							{Icon && <Icon className="size-3.5" />}
							{c.label}
						</span>
					</span>
				);
			})}
		</nav>
	);
}
