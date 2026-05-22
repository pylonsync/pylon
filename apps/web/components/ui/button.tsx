import * as React from "react";
import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

const buttonVariants = cva(
  "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-[color:var(--accent)] disabled:pointer-events-none disabled:opacity-50",
  {
    variants: {
      variant: {
        default:
          "bg-[color:var(--accent)] text-[#1a1208] hover:bg-[#ffc95a] border border-[color:var(--accent)]",
        ghost:
          "bg-transparent text-[color:var(--text)] border border-[color:var(--border-2)] hover:bg-[color:var(--bg-2)] hover:border-[#33333a]",
        outline:
          "border border-[color:var(--border-2)] bg-transparent hover:bg-[color:var(--bg-2)]",
      },
      size: {
        default: "h-9 px-3.5",
        sm: "h-[30px] px-[11px] text-[12.5px] rounded-[5px]",
        lg: "h-11 px-6",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  },
);

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {
  asChild?: boolean;
}

const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, asChild = false, ...props }, ref) => {
    // Radix's `Slot` types its rest-props against the generic
    // `HTMLElement`. Our ButtonProps narrows to HTMLButtonElement,
    // which means handlers like `onSubmit` carry the more-specific
    // `SubmitEventHandler<HTMLButtonElement>` shape. Under Next 16's
    // stricter TS check the variance flips — Slot accepts a SUPER-
    // type of handler, but a narrower handler isn't assignable. Cast
    // the rest-props to the relaxed shape so the type checker stops
    // complaining; the runtime behaviour is unchanged (Slot just
    // forwards everything to whatever child the caller passes via
    // `asChild`, and the child decides what to do with the event).
    if (asChild) {
      return (
        <Slot
          className={cn(buttonVariants({ variant, size, className }))}
          ref={ref}
          {...(props as React.HTMLAttributes<HTMLElement>)}
        />
      );
    }
    return (
      <button
        className={cn(buttonVariants({ variant, size, className }))}
        ref={ref}
        {...props}
      />
    );
  },
);
Button.displayName = "Button";

export { Button, buttonVariants };
