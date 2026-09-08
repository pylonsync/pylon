import React from "react";
import { Link, type Metadata } from "@pylonsync/react";
import { site } from "@/lib/site";

export const metadata: Metadata = {
  title: `Support — ${site.name}`,
  description: `Get help with ${site.name}.`,
};

/**
 * App Store Connect asks for a support URL, and Play requires a contact
 * address on the listing. This page is both. The answers repeat what the app
 * does so a reviewer can check a claim without installing anything.
 */
export default function SupportPage() {
  return (
    <div className="mx-auto max-w-3xl px-6 py-16 sm:py-20">
      <h1 className="text-3xl font-semibold tracking-[-0.02em]">Support</h1>
      <p className="mt-4 text-[15px] leading-relaxed text-ink-muted">
        {site.supportEmail ? (
          <>
            Email{" "}
            <a
              href={`mailto:${site.supportEmail}`}
              className="text-brand underline underline-offset-2"
            >
              {site.supportEmail}
            </a>{" "}
            and tell us what happened, what you expected, and which device you
            are on. That is usually enough to sort it out in one reply.
          </>
        ) : (
          <>Add a support address in lib/site.ts and it will appear here.</>
        )}
      </p>

      <h2 className="mt-12 text-xl font-semibold tracking-[-0.015em]">
        Common questions
      </h2>
      <dl className="mt-6 divide-y divide-line border-y border-line">
        {site.faq.map((item) => (
          <div key={item.q} className="py-5">
            <dt className="text-[15px] font-medium">{item.q}</dt>
            <dd className="mt-2 text-[14px] leading-relaxed text-ink-muted">
              {item.a}
            </dd>
          </div>
        ))}
        <div className="py-5">
          <dt className="text-[15px] font-medium">
            How do I delete my account?
          </dt>
          <dd className="mt-2 text-[14px] leading-relaxed text-ink-muted">
            In the app, open Settings and choose Delete account. It removes the
            account and its notes straight away. Cancel a subscription
            separately in the App Store or Play Store.
          </dd>
        </div>
        <div className="py-5">
          <dt className="text-[15px] font-medium">
            My notes are not showing on my other device.
          </dt>
          <dd className="mt-2 text-[14px] leading-relaxed text-ink-muted">
            Check that both devices are signed in to the same account. Notes
            written before you signed in move to the account the first time you
            sign in on that device.
          </dd>
        </div>
      </dl>

      <p className="mt-10 text-[14px] text-ink-muted">
        See also our <Link href="/privacy" className="text-brand underline underline-offset-2">privacy policy</Link>{" "}
        and{" "}
        <Link href="/terms" className="text-brand underline underline-offset-2">terms</Link>.
      </p>
    </div>
  );
}
