import * as React from "react";
import {
	Menu,
	MenuContent as MenuContentPart,
	MenuGroup,
	MenuGroupLabel as MenuGroupLabelPart,
	MenuItem as MenuItemPart,
	MenuRadioGroup,
	MenuRadioItem as MenuRadioItemPart,
	MenuItemIndicator,
	MenuSeparator as MenuSeparatorPart,
	MenuTrigger,
} from "@ui-sdk/react/menu";
import { Check } from "lucide-react";

import { cn } from "@/lib/utils";
import { popupItem, popupMotion, popupSurface } from "./popup";

type StringClassName<T> = Omit<T, "className"> & { className?: string };

/** The list of a menu. It opens under the trigger. */
function MenuContent({
	className,
	sideOffset = 4,
	...props
}: StringClassName<React.ComponentProps<typeof MenuContentPart>>) {
	return (
		<MenuContentPart
			sideOffset={sideOffset}
			className={cn(
				popupSurface,
				popupMotion,
				"max-h-(--available-height) min-w-40 overflow-y-auto p-1",
				className,
			)}
			{...props}
		/>
	);
}

/**
 * One action in a menu. `onSelect` runs the action. Pass `destructive` for an
 * action that deletes data. To make the item a link, pass the link to `render`.
 */
function MenuItem({
	className,
	...props
}: StringClassName<React.ComponentProps<typeof MenuItemPart>>) {
	return (
		<MenuItemPart
			className={cn(
				popupItem,
				"data-destructive:text-destructive data-destructive:[&_svg]:text-destructive",
				className,
			)}
			{...props}
		/>
	);
}

/** One choice in a `MenuRadioGroup`. The selected choice shows a check. */
function MenuRadioItem({
	className,
	children,
	...props
}: Omit<
	StringClassName<React.ComponentProps<typeof MenuRadioItemPart>>,
	"children"
> & { children?: React.ReactNode }) {
	return (
		<MenuRadioItemPart className={cn(popupItem, className)} {...props}>
			{children}
			<MenuItemIndicator className="ml-auto flex">
				<Check />
			</MenuItemIndicator>
		</MenuRadioItemPart>
	);
}

function MenuGroupLabel({
	className,
	...props
}: StringClassName<React.ComponentProps<typeof MenuGroupLabelPart>>) {
	return (
		<MenuGroupLabelPart
			className={cn(
				"px-2 pt-1.5 pb-1 text-[11px] font-medium text-muted-foreground",
				className,
			)}
			{...props}
		/>
	);
}

function MenuSeparator({
	className,
	...props
}: StringClassName<React.ComponentProps<typeof MenuSeparatorPart>>) {
	return (
		<MenuSeparatorPart
			className={cn("-mx-1 my-1 h-px bg-border", className)}
			{...props}
		/>
	);
}

export {
	Menu,
	MenuContent,
	MenuGroup,
	MenuGroupLabel,
	MenuItem,
	MenuRadioGroup,
	MenuRadioItem,
	MenuSeparator,
	MenuTrigger,
};
