import { Lock } from "lucide-react";

import { Button } from "@/components/ui/button";

/**
 * The content of a page that the signed-in account cannot see.
 *
 * The action is a link. Studio collects no credential: access comes from a
 * Studio operator session or an admin session of the app.
 */
export function LockedPage({
	title,
	description,
	action,
}: {
	title: string;
	description: string;
	action?: { label: string; href: string };
}) {
	return (
		<div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-1 px-4 py-14 text-center">
			<Lock className="mb-1 size-4 text-muted-foreground" />
			<h2 className="text-[13px] font-medium">{title}</h2>
			<p className="max-w-sm text-xs text-muted-foreground">{description}</p>
			{action && (
				<Button className="mt-2" render={<a href={action.href} />}>
					{action.label}
				</Button>
			)}
		</div>
	);
}
