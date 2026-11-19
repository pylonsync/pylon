import * as React from "react";
import {
  Avatar as AvatarPart,
  AvatarFallback as AvatarFallbackPart,
  AvatarImage as AvatarImagePart,
} from "@ui-sdk/react/avatar";
import { cn, type StyledProps } from "../utils";

export function Avatar({ className, ...props }: StyledProps<typeof AvatarPart>) {
  return (
    <AvatarPart
      className={cn("relative flex size-9 shrink-0 overflow-hidden rounded-full", className)}
      {...props}
    />
  );
}

export function AvatarImage({ className, ...props }: StyledProps<typeof AvatarImagePart>) {
  return <AvatarImagePart className={cn("aspect-square h-full w-full", className)} {...props} />;
}

export function AvatarFallback({ className, ...props }: StyledProps<typeof AvatarFallbackPart>) {
  return (
    <AvatarFallbackPart
      className={cn(
        "flex h-full w-full items-center justify-center rounded-full bg-muted text-sm font-medium",
        className,
      )}
      {...props}
    />
  );
}
