import { useState } from "react";
import { Lock } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import { SignInDialog } from "@/auth/SignInDialog";

export function LockedPage({
	title,
	description,
}: {
	title: string;
	description: string;
}) {
	const [open, setOpen] = useState(false);
	return (
		<>
			<Card className="mx-auto max-w-md text-center">
				<CardHeader>
					<div className="mx-auto flex size-12 items-center justify-center rounded-full bg-muted">
						<Lock className="size-5 text-muted-foreground" />
					</div>
					<CardTitle className="mt-3">{title}</CardTitle>
					<CardDescription>{description}</CardDescription>
				</CardHeader>
				<CardContent>
					<Button onClick={() => setOpen(true)}>Sign in</Button>
				</CardContent>
			</Card>
			<SignInDialog open={open} onOpenChange={setOpen} />
		</>
	);
}
