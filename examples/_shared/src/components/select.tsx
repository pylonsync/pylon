import * as React from "react";
import {
  Select as SelectPart,
  SelectContent as SelectContentPart,
  SelectGroup,
  SelectGroupLabel,
  SelectItem as SelectItemPart,
  SelectItemIndicator,
  SelectItemText,
  SelectSeparator as SelectSeparatorPart,
  SelectTrigger as SelectTriggerPart,
  SelectValue,
} from "@ui-sdk/react/select";
import { Check, ChevronDown } from "lucide-react";
import { cn, type StyledProps } from "../utils";

export { SelectGroup, SelectValue };

export interface SelectOption {
  value: string;
  label: string;
  disabled?: boolean;
}

export interface SelectProps {
  /**
   * The options as data. The trigger reads the label of the selected value
   * from it, so pass it whenever a value is selected before the list opens.
   */
  items?: SelectOption[];
  /** The selected value. Pass it with `onValueChange` to control the select. */
  value?: string | null;
  defaultValue?: string | null;
  onValueChange?: (value: string) => void;
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  /** The text the trigger shows while nothing is selected. */
  placeholder?: string;
  /** The form field name. Without a name the select submits nothing. */
  name?: string;
  required?: boolean;
  disabled?: boolean;
  children?: React.ReactNode;
}

/**
 * Pick one value from a list. Compose it from `SelectTrigger`, `SelectValue`,
 * `SelectContent` and one `SelectItem` for each option.
 */
export function Select({ onValueChange, onOpenChange, ...props }: SelectProps) {
  return (
    <SelectPart
      onValueChange={(value: string | null) => {
        if (value != null) onValueChange?.(value);
      }}
      onOpenChange={(open: boolean) => onOpenChange?.(open)}
      {...props}
    />
  );
}

export function SelectTrigger({
  className,
  children,
  ...props
}: Omit<StyledProps<typeof SelectTriggerPart>, "children"> & { children?: React.ReactNode }) {
  return (
    <SelectTriggerPart
      className={cn(
        "flex h-9 w-full items-center justify-between gap-2 whitespace-nowrap rounded-md border border-input bg-background px-3 py-2 text-left text-sm ring-offset-background focus:outline-none focus-visible:ring-1 focus-visible:ring-ring data-disabled:cursor-not-allowed data-disabled:opacity-50 data-placeholder:text-muted-foreground [&>span]:truncate",
        className,
      )}
      {...props}
    >
      {children}
      <ChevronDown aria-hidden="true" className="size-4 shrink-0 opacity-50" />
    </SelectTriggerPart>
  );
}

export function SelectContent({
  className,
  sideOffset = 4,
  ...props
}: StyledProps<typeof SelectContentPart>) {
  return (
    <SelectContentPart
      sideOffset={sideOffset}
      className={cn(
        "z-50 max-h-(--available-height) min-w-(--anchor-width) overflow-y-auto rounded-md border bg-popover p-1 text-popover-foreground shadow-md outline-none",
        "origin-(--transform-origin) transition-[opacity,scale] duration-150 data-starting-style:scale-95 data-starting-style:opacity-0 data-ending-style:scale-95 data-ending-style:opacity-0 motion-reduce:transition-none",
        className,
      )}
      {...props}
    />
  );
}

/**
 * One option. When its children are not plain text, set `label` to the text
 * the trigger shows for it.
 */
export function SelectItem({
  className,
  children,
  ...props
}: Omit<StyledProps<typeof SelectItemPart>, "children"> & { children?: React.ReactNode }) {
  return (
    <SelectItemPart
      className={cn(
        "relative flex w-full cursor-default select-none items-center rounded-sm py-1.5 pl-8 pr-2 text-sm outline-none data-highlighted:bg-accent data-highlighted:text-accent-foreground data-disabled:pointer-events-none data-disabled:opacity-50",
        className,
      )}
      {...props}
    >
      <span className="absolute left-2 flex size-3.5 items-center justify-center">
        <SelectItemIndicator>
          <Check aria-hidden="true" className="size-4" />
        </SelectItemIndicator>
      </span>
      <SelectItemText>{children}</SelectItemText>
    </SelectItemPart>
  );
}

/** A heading for the `SelectGroup` around it. */
export function SelectLabel({ className, ...props }: StyledProps<typeof SelectGroupLabel>) {
  return <SelectGroupLabel className={cn("px-2 py-1.5 text-sm font-semibold", className)} {...props} />;
}

export function SelectSeparator({ className, ...props }: StyledProps<typeof SelectSeparatorPart>) {
  return <SelectSeparatorPart className={cn("-mx-1 my-1 h-px bg-muted", className)} {...props} />;
}
