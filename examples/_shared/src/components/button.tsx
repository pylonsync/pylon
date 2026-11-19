import * as React from "react";
import { Button as ButtonPart } from "@ui-sdk/react/button";
import { cva, type VariantProps } from "class-variance-authority";
import { cn, type StyledProps } from "../utils";

const buttonVariants = cva(
  "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring data-disabled:pointer-events-none data-disabled:opacity-50 data-pending:cursor-progress data-pending:opacity-70 [&_svg]:pointer-events-none [&_svg:not([class*='size-'])]:size-4 [&_svg]:shrink-0",
  {
    variants: {
      variant: {
        default: "bg-primary text-primary-foreground hover:bg-primary/90",
        destructive:
          "bg-destructive text-destructive-foreground hover:bg-destructive/90",
        outline:
          "border border-input bg-background hover:bg-accent hover:text-accent-foreground",
        secondary:
          "bg-secondary text-secondary-foreground hover:bg-secondary/80",
        ghost: "hover:bg-accent hover:text-accent-foreground",
        link: "text-primary underline-offset-4 hover:underline",
      },
      size: {
        default: "h-9 px-4 py-2",
        sm: "h-8 rounded-md px-3 text-xs",
        lg: "h-10 rounded-md px-6",
        icon: "h-9 w-9",
        xs: "h-7 rounded px-2 text-xs",
      },
    },
    defaultVariants: { variant: "default", size: "default" },
  },
);

export type ButtonProps = StyledProps<typeof ButtonPart> & VariantProps<typeof buttonVariants>;

/**
 * A button. Its `type` is "button" unless set, so a form's submit button needs
 * `type="submit"`. When `onClick` returns a Promise the button is pending
 * (`data-pending`) until the Promise settles, and a second press does nothing.
 *
 * To render a link that looks like a button, pass the link to `render`:
 * `<Button render={<a href="/" />}>Home</Button>`.
 */
export function Button({ className, variant, size, ...props }: ButtonProps) {
  return <ButtonPart className={cn(buttonVariants({ variant, size }), className)} {...props} />;
}

export { buttonVariants };
