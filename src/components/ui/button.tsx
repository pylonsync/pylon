import * as React from "react";
import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "../../lib/utils";

// Buttons (v2). Primary is solid ink (PS-style — high contrast, brand-
// neutral). Default is hairline outline on paper. Subtle pill for tags
// and tertiary slots. Sizes are tighter than stock shadcn for dashboard
// density. Soft shadow under the primary so it feels elevated.
const buttonVariants = cva(
  "inline-flex items-center justify-center gap-1.5 whitespace-nowrap rounded-[var(--radius-md)] text-[13px] font-medium tracking-tight transition-[background-color,border-color,color,box-shadow,scale] duration-150 ease-out active:scale-[0.96] motion-reduce:active:scale-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-brand)] focus-visible:ring-offset-2 focus-visible:ring-offset-[var(--color-paper)] disabled:pointer-events-none disabled:opacity-40 [&_svg]:size-3.5 [&_svg]:shrink-0",
  {
    variants: {
      variant: {
        // Primary action — solid ink (near-black). PS-style, used for
        // the canonical CTA per view (Connect, Create, Save).
        primary:
          "bg-[var(--color-ink)] text-[var(--color-paper)] hover:opacity-90 border border-[var(--color-ink)] shadow-[var(--shadow-card)]",
        // Default = hairline outline. The workhorse for most actions.
        default:
          "bg-[var(--color-paper)] text-[var(--color-ink)] hover:bg-[var(--color-paper-1)] border border-[var(--color-rule)] shadow-[var(--shadow-card)]",
        // Outline (alias of default for shadcn compat)
        outline:
          "bg-transparent text-[var(--color-ink)] hover:bg-[var(--color-paper-1)] border border-[var(--color-rule)]",
        // Ghost — no chrome until hover. Toolbar / icon buttons.
        ghost:
          "bg-transparent text-[var(--color-ink-2)] hover:bg-[var(--color-paper-1)] hover:text-[var(--color-ink)] border border-transparent",
        // Subtle — sunken pill, like a tag or filter
        subtle:
          "bg-[var(--color-paper-1)] text-[var(--color-ink)] hover:bg-[var(--color-paper-2)] border border-transparent",
        // Cobalt — brand accent for places where the ink primary would
        // be too heavy (e.g. an inline-cta in a card alongside neutral
        // text). Sparingly.
        cobalt:
          "bg-[var(--color-brand)] text-[var(--color-brand-fg)] hover:bg-[var(--color-brand-hover)] border border-[var(--color-brand)]",
        // Destructive
        destructive:
          "bg-[var(--color-status-fail)] text-white hover:opacity-90 border border-[var(--color-status-fail)]",
        // Link — looks like text, behaves like a button
        link: "bg-transparent text-[var(--color-brand)] hover:underline underline-offset-4 border-0 p-0 h-auto active:scale-100",
      },
      size: {
        xs: "h-7 px-2 text-[12px] [&_svg]:size-3",
        sm: "h-8 px-3 text-[12.5px]",
        default: "h-9 px-4",
        lg: "h-10 px-5 text-[13.5px]",
        icon: "size-8 px-0",
        "icon-sm": "size-7 px-0",
        full: "h-9 px-4 w-full",
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
  /** Disable the scale-on-press feedback (for contexts where motion distracts). */
  static?: boolean;
}

const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, asChild = false, static: isStatic = false, ...props }, ref) => {
    const Comp = asChild ? Slot : "button";
    return (
      <Comp
        className={cn(
          buttonVariants({ variant, size }),
          isStatic && "active:scale-100",
          className,
        )}
        ref={ref}
        {...props}
      />
    );
  },
);
Button.displayName = "Button";

export { Button, buttonVariants };
