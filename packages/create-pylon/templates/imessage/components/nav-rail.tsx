import React from "react";
import { Link } from "@pylonsync/react";
import { LogOut, MessageCircle, Settings2, Users } from "lucide-react";
import { cn } from "@/lib/utils";

export type View = "messages" | "contacts" | "setup";

const ITEMS: { view: View; href: string; label: string; icon: typeof MessageCircle }[] = [
  { view: "messages", href: "/", label: "Messages", icon: MessageCircle },
  { view: "contacts", href: "/contacts", label: "Allowlist", icon: Users },
  { view: "setup", href: "/setup", label: "Setup", icon: Settings2 },
];

/** Left rail on desktop, bottom tab bar on phones. */
export function NavRail({
  view,
  setupIssues,
  requests,
  onSignOut,
}: {
  view: View;
  setupIssues: number;
  requests: number;
  onSignOut: () => void;
}) {
  return (
    <nav
      aria-label="Sections"
      className="order-last flex shrink-0 items-center justify-around border-t bg-rail/80 px-2 py-1.5 md:order-first md:w-[68px] md:flex-col md:justify-start md:gap-1 md:border-r md:border-t-0 md:px-0 md:py-4"
    >
      <span className="mb-4 hidden size-9 items-center justify-center rounded-[0.7rem] bg-gradient-to-b from-[#5ac8fa] to-[#0a84ff] text-white shadow-[inset_0_1px_0_rgba(255,255,255,0.35),0_4px_12px_-4px_rgba(10,132,255,0.6)] md:flex">
        <MessageCircle className="size-[18px] fill-white" strokeWidth={1.5} />
      </span>
      {ITEMS.map(({ view: v, href, label, icon: Icon }) => {
        const badge = v === "setup" ? setupIssues : v === "contacts" ? requests : 0;
        const active = v === view;
        return (
          <Link
            key={v}
            href={href}
            aria-current={active ? "page" : undefined}
            className={cn(
              "relative flex flex-col items-center gap-0.5 rounded-xl px-3 py-1.5 text-[10px] font-medium transition-colors duration-300 md:w-[58px] md:py-2",
              active ? "text-bubble-out md:bg-bubble-out/10" : "text-muted-foreground hover:text-foreground md:hover:bg-black/[0.04]",
            )}
          >
            <Icon className="size-5" strokeWidth={1.5} />
            {label}
            {badge > 0 ? (
              <span className="absolute right-2 top-0.5 min-w-4 rounded-full bg-[#ff3b30] px-1 text-center text-[10px] font-semibold leading-4 text-white md:right-1 md:top-1">
                {badge}
              </span>
            ) : null}
          </Link>
        );
      })}
      <button
        type="button"
        onClick={onSignOut}
        className="flex flex-col items-center gap-0.5 whitespace-nowrap rounded-xl px-3 py-1.5 text-[10px] font-medium text-muted-foreground hover:text-foreground md:mt-auto md:w-[58px]"
      >
        <LogOut className="size-5" strokeWidth={1.5} />
        Sign out
      </button>
    </nav>
  );
}
