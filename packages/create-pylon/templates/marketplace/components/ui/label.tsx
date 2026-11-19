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
        "flex items-center gap-2 text-sm font-medium leading-none select-none group-data-[disabled=true]:pointer-events-none group-data-[disabled=true]:opacity-50 peer-disabled:cursor-not-allowed peer-disabled:opacity-50",
        className,
      )}
      {...props}
    />
  );
}

export { Label };
