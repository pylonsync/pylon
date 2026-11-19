import * as React from "react";
import { Label as LabelPart } from "@ui-sdk/react/label";
import { cn, type StyledProps } from "../utils";

export function Label({ className, ...props }: StyledProps<typeof LabelPart>) {
  return (
    <LabelPart
      className={cn(
        "text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70",
        className,
      )}
      {...props}
    />
  );
}
