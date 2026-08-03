import { Lock } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";

/**
 * "You can't see this" panel.
 *
 * The action is a plain link, not a sign-in dialog. Studio has no credential
 * of its own to collect — access comes from being signed in to the app as an
 * admin — so the only useful thing to offer is a way back to that sign-in.
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
		<Card className="mx-auto max-w-md text-center">
			<CardHeader>
				<div className="mx-auto flex size-12 items-center justify-center rounded-full bg-muted">
					<Lock className="size-5 text-muted-foreground" />
				</div>
				<CardTitle className="mt-3">{title}</CardTitle>
				<CardDescription>{description}</CardDescription>
			</CardHeader>
			{action && (
				<CardContent>
					<Button asChild>
						<a href={action.href}>{action.label}</a>
					</Button>
				</CardContent>
			)}
		</Card>
	);
}
