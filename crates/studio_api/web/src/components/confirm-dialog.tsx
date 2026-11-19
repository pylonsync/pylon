import * as React from "react";

import { Button } from "@/components/ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";

export interface ConfirmDialogProps {
	open: boolean;
	title: React.ReactNode;
	description?: React.ReactNode;
	/** The text of the button that confirms. It names the action. */
	confirmLabel: string;
	/** The text of the button that closes the dialog. The default is "Cancel". */
	cancelLabel?: string;
	/** The action deletes data or stops something. The button is red. */
	destructive?: boolean;
	/** Runs the action. The dialog stays open until a returned Promise settles. */
	onConfirm: () => unknown;
	onCancel: () => void;
}

/** A question with a Cancel button and one button that runs the action. */
export function ConfirmDialog({
	open,
	title,
	description,
	confirmLabel,
	cancelLabel = "Cancel",
	destructive,
	onConfirm,
	onCancel,
}: ConfirmDialogProps) {
	return (
		<Dialog open={open} onOpenChange={(next) => !next && onCancel()}>
			<DialogContent>
				<DialogHeader>
					<DialogTitle>{title}</DialogTitle>
					{description && <DialogDescription>{description}</DialogDescription>}
				</DialogHeader>
				<DialogFooter className="border-t-0 bg-transparent pt-0 pb-3.5">
					<Button variant="outline" onClick={onCancel}>
						{cancelLabel}
					</Button>
					<Button
						variant={destructive ? "destructive" : "default"}
						onClick={onConfirm}
					>
						{confirmLabel}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
