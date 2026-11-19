import * as React from "react";
import {
	Popover,
	PopoverClose,
	PopoverContent as PopoverContentPart,
	PopoverTitle as PopoverTitlePart,
	PopoverTrigger,
} from "@ui-sdk/react/popover";

import { cn } from "@/lib/utils";
import { popupMotion, popupSurface } from "./popup";

type StringClassName<T> = Omit<T, "className"> & { className?: string };

/** The panel of a popover. It opens next to the trigger. */
function PopoverContent({
	className,
	sideOffset = 4,
	...props
}: StringClassName<React.ComponentProps<typeof PopoverContentPart>>) {
	return (
		<PopoverContentPart
			sideOffset={sideOffset}
			className={cn(popupSurface, popupMotion, "p-3", className)}
			{...props}
		/>
	);
}

function PopoverTitle({
	className,
	...props
}: StringClassName<React.ComponentProps<typeof PopoverTitlePart>>) {
	return (
		<PopoverTitlePart
			className={cn("text-[13px] font-semibold", className)}
			{...props}
		/>
	);
}

export { Popover, PopoverClose, PopoverContent, PopoverTitle, PopoverTrigger };
