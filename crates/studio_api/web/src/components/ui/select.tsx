import * as React from "react";
import {
	Select as SelectRoot,
	SelectContent,
	SelectGroup,
	SelectGroupLabel,
	SelectItem,
	SelectItemIndicator,
	SelectItemText,
	SelectSeparator,
	SelectTrigger,
	SelectValue,
	type ListData,
} from "@ui-sdk/react/select";
import { Check, ChevronsUpDown } from "lucide-react";

import { cn } from "@/lib/utils";
import { popupItem, popupMotion, popupSurface } from "./popup";

export interface SelectOption {
	value: string;
	label: string;
	disabled?: boolean;
}

export interface SelectOptionGroup {
	label: string;
	options: SelectOption[];
}

export interface SelectProps {
	/** The options in display order. Mix options and groups of options. */
	options: Array<SelectOption | SelectOptionGroup>;
	/** The selected value. `null` selects nothing. */
	value: string | null;
	onValueChange: (value: string) => void;
	/** The text the trigger shows while nothing is selected. */
	placeholder?: string;
	disabled?: boolean;
	id?: string;
	"aria-label"?: string;
	/** Classes for the trigger. */
	className?: string;
}

function isGroup(o: SelectOption | SelectOptionGroup): o is SelectOptionGroup {
	return "options" in o;
}

function Option({ option }: { option: SelectOption }) {
	return (
		<SelectItem
			value={option.value}
			disabled={option.disabled}
			className={cn(popupItem, "justify-between")}
		>
			<SelectItemText>{option.label}</SelectItemText>
			<SelectItemIndicator className="flex">
				<Check />
			</SelectItemIndicator>
		</SelectItem>
	);
}

/**
 * Pick one value from a list. Pass the options as data. The component renders
 * the trigger, the list, the keyboard navigation and the typeahead.
 */
function Select({
	options,
	value,
	onValueChange,
	placeholder,
	disabled,
	id,
	"aria-label": ariaLabel,
	className,
}: SelectProps) {
	const items = React.useMemo<ListData>(
		() =>
			options.map((o) =>
				isGroup(o)
					? { type: "group" as const, label: o.label, items: o.options }
					: o,
			),
		[options],
	);
	return (
		<SelectRoot
			items={items}
			value={value}
			placeholder={placeholder}
			disabled={disabled}
			onValueChange={(next) => {
				if (typeof next === "string") onValueChange(next);
			}}
		>
			<SelectTrigger
				id={id}
				aria-label={ariaLabel}
				className={cn(
					"flex h-7 min-w-0 cursor-default items-center justify-between gap-1.5 rounded-md border border-border-strong bg-background pr-1.5 pl-2 text-left text-[13px] transition-colors duration-100 hover:bg-accent",
					"data-disabled:pointer-events-none data-disabled:opacity-50 data-placeholder:text-muted-foreground",
					className,
				)}
			>
				<SelectValue className="truncate" />
				<ChevronsUpDown className="size-3 shrink-0 text-muted-foreground" />
			</SelectTrigger>
			<SelectContent
				sideOffset={4}
				className={cn(
					popupSurface,
					popupMotion,
					"max-h-(--available-height) min-w-(--anchor-width) overflow-y-auto p-1",
				)}
			>
				{options.map((o, i) =>
					isGroup(o) ? (
						<React.Fragment key={`group-${o.label}`}>
							{i > 0 && <SelectSeparator className="-mx-1 my-1 h-px bg-border" />}
							<SelectGroup>
								<SelectGroupLabel className="px-2 pt-1.5 pb-1 text-[11px] font-medium text-muted-foreground">
									{o.label}
								</SelectGroupLabel>
								{o.options.map((option) => (
									<Option key={option.value} option={option} />
								))}
							</SelectGroup>
						</React.Fragment>
					) : (
						<Option key={o.value} option={o} />
					),
				)}
			</SelectContent>
		</SelectRoot>
	);
}

export { Select };
