import * as React from "react";
import { Tabs, TabsList as TabsListPart, TabsPanel, TabsTab } from "@ui-sdk/react/tabs";
import { cn, type StyledProps } from "../utils";

export { Tabs };

export function TabsList({ className, ...props }: StyledProps<typeof TabsListPart>) {
  return (
    <TabsListPart
      className={cn(
        "inline-flex h-9 items-center justify-center rounded-lg bg-muted p-1 text-muted-foreground",
        className,
      )}
      {...props}
    />
  );
}

/** A tab. Its `value` pairs it with the `TabsContent` of the same value. */
export function TabsTrigger({ className, ...props }: StyledProps<typeof TabsTab>) {
  return (
    <TabsTab
      className={cn(
        "inline-flex items-center justify-center whitespace-nowrap rounded-md px-3 py-1 text-sm font-medium ring-offset-background transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 data-disabled:pointer-events-none data-disabled:opacity-50 data-selected:bg-background data-selected:text-foreground data-selected:shadow",
        className,
      )}
      {...props}
    />
  );
}

/** The panel of the tab with the same `value`. */
export function TabsContent({ className, ...props }: StyledProps<typeof TabsPanel>) {
  return (
    <TabsPanel
      className={cn(
        "mt-2 ring-offset-background focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
        className,
      )}
      {...props}
    />
  );
}
