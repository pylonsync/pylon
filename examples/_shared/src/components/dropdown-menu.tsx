import * as React from "react";
import {
  Menu as DropdownMenu,
  MenuCheckboxItem,
  MenuContent,
  MenuGroup as DropdownMenuGroup,
  MenuGroupLabel,
  MenuItem,
  MenuItemIndicator,
  MenuRadioGroup as DropdownMenuRadioGroup,
  MenuRadioItem,
  MenuSeparator,
  MenuSub as DropdownMenuSub,
  MenuSubContent,
  MenuSubTrigger,
  MenuTrigger as DropdownMenuTrigger,
} from "@ui-sdk/react/menu";
import { Check, ChevronRight, Circle } from "lucide-react";
import { cn, type StyledProps } from "../utils";

export {
  DropdownMenu,
  DropdownMenuGroup,
  DropdownMenuRadioGroup,
  DropdownMenuSub,
  DropdownMenuTrigger,
};

const contentClasses =
  "z-50 min-w-[8rem] overflow-y-auto rounded-md border bg-popover p-1 text-popover-foreground outline-none max-h-(--available-height) origin-(--transform-origin) transition-[opacity,scale] duration-150 data-starting-style:scale-95 data-starting-style:opacity-0 data-ending-style:scale-95 data-ending-style:opacity-0 motion-reduce:transition-none";

const itemClasses =
  "relative flex cursor-default select-none items-center rounded-sm py-1.5 text-sm outline-none transition-colors data-highlighted:bg-accent data-highlighted:text-accent-foreground data-disabled:pointer-events-none data-disabled:opacity-50";

export function DropdownMenuSubTrigger({
  className,
  inset,
  children,
  ...props
}: Omit<StyledProps<typeof MenuSubTrigger>, "children"> & {
  inset?: boolean;
  children?: React.ReactNode;
}) {
  return (
    <MenuSubTrigger
      className={cn(
        "flex cursor-default select-none items-center gap-2 rounded-sm px-2 py-1.5 text-sm outline-none data-highlighted:bg-accent data-open:bg-accent",
        inset && "pl-8",
        className,
      )}
      {...props}
    >
      {children}
      <ChevronRight aria-hidden="true" className="ml-auto size-4" />
    </MenuSubTrigger>
  );
}

export function DropdownMenuSubContent({ className, ...props }: StyledProps<typeof MenuSubContent>) {
  return <MenuSubContent className={cn(contentClasses, "shadow-lg", className)} {...props} />;
}

export function DropdownMenuContent({
  className,
  sideOffset = 4,
  ...props
}: StyledProps<typeof MenuContent>) {
  return (
    <MenuContent
      sideOffset={sideOffset}
      className={cn(contentClasses, "shadow-md", className)}
      {...props}
    />
  );
}

/** A menu action. `onSelect` runs when the item is chosen by pointer or keyboard. */
export function DropdownMenuItem({
  className,
  inset,
  ...props
}: StyledProps<typeof MenuItem> & { inset?: boolean }) {
  return (
    <MenuItem
      className={cn(itemClasses, "gap-2 px-2 [&_svg]:size-4 [&_svg]:shrink-0", inset && "pl-8", className)}
      {...props}
    />
  );
}

export function DropdownMenuCheckboxItem({
  className,
  children,
  ...props
}: Omit<StyledProps<typeof MenuCheckboxItem>, "children"> & { children?: React.ReactNode }) {
  return (
    <MenuCheckboxItem className={cn(itemClasses, "pl-8 pr-2", className)} {...props}>
      <span className="absolute left-2 flex size-3.5 items-center justify-center">
        <MenuItemIndicator>
          <Check aria-hidden="true" className="size-4" />
        </MenuItemIndicator>
      </span>
      {children}
    </MenuCheckboxItem>
  );
}

export function DropdownMenuRadioItem({
  className,
  children,
  ...props
}: Omit<StyledProps<typeof MenuRadioItem>, "children"> & { children?: React.ReactNode }) {
  return (
    <MenuRadioItem className={cn(itemClasses, "pl-8 pr-2", className)} {...props}>
      <span className="absolute left-2 flex size-3.5 items-center justify-center">
        <MenuItemIndicator>
          <Circle aria-hidden="true" className="size-2 fill-current" />
        </MenuItemIndicator>
      </span>
      {children}
    </MenuRadioItem>
  );
}

/** Text in a menu that is not an item, such as the signed-in account. */
export function DropdownMenuLabel({
  className,
  inset,
  ...props
}: React.HTMLAttributes<HTMLDivElement> & { inset?: boolean }) {
  return (
    <div className={cn("px-2 py-1.5 text-sm font-semibold", inset && "pl-8", className)} {...props} />
  );
}

/** The name of the `DropdownMenuGroup` or `DropdownMenuRadioGroup` around it. */
export function DropdownMenuGroupLabel({
  className,
  inset,
  ...props
}: StyledProps<typeof MenuGroupLabel> & { inset?: boolean }) {
  return (
    <MenuGroupLabel
      className={cn("px-2 py-1.5 text-sm font-semibold", inset && "pl-8", className)}
      {...props}
    />
  );
}

export function DropdownMenuSeparator({ className, ...props }: StyledProps<typeof MenuSeparator>) {
  return <MenuSeparator className={cn("-mx-1 my-1 h-px bg-muted", className)} {...props} />;
}

export function DropdownMenuShortcut({ className, ...props }: React.HTMLAttributes<HTMLSpanElement>) {
  return <span className={cn("ml-auto text-xs tracking-widest opacity-60", className)} {...props} />;
}
