import * as React from "react";
import { Label as LabelPart } from "@ui-sdk/react/label";

import { cn } from "@/lib/utils";

function Label({
  className,
  ...props
}: Omit<React.ComponentProps<typeof LabelPart>, "className"> & {
  className?: string;
}) {
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

export { Label };
