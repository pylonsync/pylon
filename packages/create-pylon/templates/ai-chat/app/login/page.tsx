import React from "react";
import { Link, type Metadata, type PageProps } from "@pylonsync/react";
import { AuthForm } from "../auth-form";
import { BrandMark } from "@/components/chat/icons";
import { siteConfig } from "@/lib/site.config";
import { brandIcon } from "@/lib/brand-icon";
import { safeNextPath } from "@/lib/chat";

export const metadata: Metadata = {
  title: `Sign in to ${siteConfig.brand.name}`,
  robots: "noindex",
  icons: { icon: brandIcon },
};

// `/login`: optional. Chat works without an account; signing in keeps the
// history on every device, and the guest's chats move to the account. A
// signed-in account is sent back to the chat.
export default function LoginPage({ auth, response, searchParams }: PageProps) {
  const next = safeNextPath(typeof searchParams.next === "string" ? searchParams.next : null);
  if (auth.user_id && !auth.is_guest && !auth.user_id.startsWith("guest_")) response.redirect(next);
  const { brand } = siteConfig;
  const hasGuestSession = Boolean(auth.user_id);

  return (
    <div className="flex min-h-[100dvh] flex-col bg-paper px-4 py-6 sm:px-6">
      <Link href={next} className="press inline-flex w-max items-center gap-2.5 rounded-xl px-1.5 py-1 hover:bg-hover">
        <BrandMark size={26} />
        <span className="text-[15.5px] font-semibold tracking-[-0.015em] text-ink">{brand.name}</span>
      </Link>

      <div className="flex flex-1 items-center justify-center py-10">
        <div className="rise w-full max-w-[25rem]">
          <h1 className="text-center font-serif text-[32px] leading-tight tracking-[-0.02em] text-ink">
            Sign in to {brand.name}
          </h1>
          <p className="mx-auto mt-2 max-w-[21rem] text-center text-[14.5px] leading-relaxed text-ink-2">
            {hasGuestSession
              ? "Your chats from this browser move to your account."
              : "Keep your chats on every device."}
          </p>
          <div className="bezel mt-8">
            <div className="bezel-core p-5 sm:p-6">
              <AuthForm next={next} />
            </div>
          </div>
          <p className="mt-6 text-center text-[13px] text-ink-3">
            <Link href={next} className="underline decoration-ink/20 underline-offset-4 hover:text-ink">
              Continue without an account
            </Link>
          </p>
        </div>
      </div>
    </div>
  );
}
