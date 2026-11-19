import { useRef, useState, type ReactNode } from "react";
import { X } from "lucide-react";

import {
	Sheet,
	SheetBody,
	SheetClose,
	SheetContent,
	SheetDescription,
	SheetFooter,
	SheetHeader,
	SheetTitle,
} from "@/components/ui/sheet";
import { useEscapeOutside } from "@/hooks/use-escape";

export interface DetailSheetProps {
	open: boolean;
	title: ReactNode;
	/** A second line under the title, such as an id. */
	subtitle?: ReactNode;
	onClose: () => void;
	/** Buttons at the bottom of the panel. */
	footer?: ReactNode;
	children?: ReactNode;
}

/**
 * A read-only panel on the right edge for one item of a list: a request, a
 * job, a workflow run. The list behind it stays usable. Escape closes it.
 */
export function DetailSheet({
	open,
	title,
	subtitle,
	onClose,
	footer,
	children,
}: DetailSheetProps) {
	const panel = useRef<HTMLDivElement>(null);
	useEscapeOutside(open, panel, onClose);
	// The last content stays rendered while the sheet animates out.
	const [shown, setShown] = useState({ title, subtitle, footer, children });
	if (open && (shown.children !== children || shown.footer !== footer || shown.title !== title)) {
		setShown({ title, subtitle, footer, children });
	}
	return (
		<Sheet open={open} modal={false} onOpenChange={(next) => !next && onClose()}>
			<SheetContent
				ref={panel}
				side="right"
				backdrop={false}
				initialFocus="content"
				className="w-[min(520px,100vw)]"
			>
				<SheetHeader className="justify-between">
					<div className="flex min-w-0 flex-col">
						<SheetTitle>{shown.title}</SheetTitle>
						{shown.subtitle && (
							<SheetDescription className="truncate font-mono text-[11px]">
								{shown.subtitle}
							</SheetDescription>
						)}
					</div>
					<SheetClose
						aria-label="Close"
						className="flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground"
					>
						<X className="size-3.5" />
					</SheetClose>
				</SheetHeader>
				<SheetBody>{shown.children}</SheetBody>
				{shown.footer && <SheetFooter>{shown.footer}</SheetFooter>}
			</SheetContent>
		</Sheet>
	);
}
