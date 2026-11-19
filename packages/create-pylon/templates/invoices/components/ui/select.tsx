"use client";

import * as React from "react";
import {
  Select as SelectRoot,
  SelectContent,
  SelectItem,
  SelectItemIndicator,
  SelectItemText,
  SelectTrigger,
  SelectValue,
} from "@ui-sdk/react/select";
import { Check, ChevronDown } from "lucide-react";

import { cn } from "@/lib/utils";

export interface SelectOption {
  value: string;
  label: string;
  disabled?: boolean;
}

export interface SelectProps {
  /** The options, in display order. */
  items: SelectOption[];
  /** The selected value. Pass it with `onValueChange` to control the select. */
  value?: string | null;
  defaultValue?: string | null;
  onValueChange?: (value: string) => void;
  /** The text the trigger shows while nothing is selected. */
  placeholder?: string;
  /** The form field name. Without a name the select submits nothing. */
  name?: string;
  required?: boolean;
  disabled?: boolean;
  id?: string;
  "aria-label"?: string;
  /** Classes for the trigger. */
  className?: string;
}

/**
 * Pick one value from a list. Give it the options as data; it renders the
 * trigger, the list, keyboard navigation and typeahead.
 *
 * For custom option content, groups or search, compose the parts from
 * `@ui-sdk/react/select` in a new component next to this one.
 */
function Select({
  items,
  onValueChange,
  id,
  "aria-label": ariaLabel,
  className,
  ...props
}: SelectProps) {
  return (
    <SelectRoot
      items={items}
      onValueChange={(value) => onValueChange?.(value ?? "")}
      {...props}
    >
      <SelectTrigger
        id={id}
        aria-label={ariaLabel}
        className={cn(
          "flex h-9 w-full items-center justify-between gap-2 rounded-md border border-input bg-transparent py-1 pl-3 pr-2.5 text-left text-base shadow-sm md:text-[13px] transition-colors",
          "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
          "data-disabled:cursor-not-allowed data-disabled:opacity-50 data-placeholder:text-muted-foreground",
          className,
        )}
      >
        <SelectValue className="truncate" />
        <ChevronDown className="size-4 shrink-0 opacity-50" />
      </SelectTrigger>
      <SelectContent
        sideOffset={4}
        className={cn(
          "z-50 max-h-(--available-height) min-w-(--anchor-width) overflow-y-auto rounded-md border bg-popover p-1 text-popover-foreground shadow-md outline-none",
          "origin-(--transform-origin) transition-[opacity,scale] duration-150 data-starting-style:scale-95 data-starting-style:opacity-0 data-ending-style:scale-95 data-ending-style:opacity-0 motion-reduce:transition-none",
        )}
      >
        {items.map((item) => (
          <SelectItem
            key={item.value}
            value={item.value}
            disabled={item.disabled}
            className="flex cursor-default select-none items-center justify-between gap-2 rounded-sm px-2 py-1.5 text-sm outline-none data-highlighted:bg-accent data-highlighted:text-accent-foreground data-disabled:pointer-events-none data-disabled:opacity-50"
          >
            <SelectItemText>{item.label}</SelectItemText>
            <SelectItemIndicator>
              <Check className="size-4" />
            </SelectItemIndicator>
          </SelectItem>
        ))}
      </SelectContent>
    </SelectRoot>
  );
}

export { Select };
