import React from "react";
import { type Metadata } from "@pylonsync/react";
import { Company, LegalPage } from "@/components/legal";
import { site } from "@/lib/site";

export const metadata: Metadata = {
  title: `Terms of Service — ${site.name}`,
  description: `The agreement between you and ${site.name}.`,
};

/**
 * The terms the app links to from the paywall and from Settings. Apple
 * requires subscription terms to be visible next to the purchase, and this
 * page is where the detail lives. Keep the billing section true to what your
 * products actually do.
 */
export default function TermsPage() {
  return (
    <LegalPage title="Terms of Service">
      <p>
        These terms are the agreement between you and <Company /> for the use
        of {site.name}. Using the app means you accept them.
      </p>

      <h2>Who may use it</h2>
      <p>
        You need to be at least 13 years old. If you use {site.name} for an
        organization, you confirm you may accept these terms for it.
      </p>

      <h2>Your account</h2>
      <p>
        You are responsible for what happens under your account, so keep
        access to your email and devices secure. Tell us if you think someone
        else has your account.
      </p>

      <h2>Free and paid</h2>
      <p>
        The free tier holds up to {site.pricing.freeLimit} notes. A
        subscription lifts that limit and includes what we add later. We may
        change what each tier includes, and we will not reduce what you have
        already paid for during a period you have paid.
      </p>

      <h2>Billing</h2>
      <ul>
        <li>
          Apple and Google sell and bill the subscription, not us. The price
          you see in the app is the price they charge in your country.
        </li>
        <li>
          A subscription renews automatically until you cancel it. Cancel at
          least 24 hours before the period ends, in your App Store or Play
          Store account settings.
        </li>
        <li>
          Refunds follow the store&apos;s policy. We cannot issue a refund for
          a purchase we did not process, so ask Apple or Google.
        </li>
        <li>
          Deleting the app does not cancel a subscription. Cancel it in the
          store.
        </li>
      </ul>

      <h2>Your content</h2>
      <p>
        Your notes are yours. You give us only the permission we need to run
        the service, which is to store your content, back it up, and send it to
        the devices you sign in on. We claim no ownership of it and we do not
        use it to train anything.
      </p>
      <p>
        You are responsible for what you put in the app, and for having the
        right to put it there.
      </p>

      <h2>Acceptable use</h2>
      <p>Do not use {site.name} to:</p>
      <ul>
        <li>break the law, or store material that is illegal to hold;</li>
        <li>
          attack the service, work around its limits, or get at other
          people&apos;s data;
        </li>
        <li>resell it as your own product.</li>
      </ul>

      <h2>Availability</h2>
      <p>
        We work to keep {site.name} running, but we do not promise it will be
        available without interruption. We may change or stop features. If we
        shut the service down, we will give reasonable notice and a way to
        export your notes.
      </p>

      <h2>Ending it</h2>
      <p>
        You can stop at any time by deleting your account in Settings. We may
        suspend or close an account that breaks these terms or puts the service
        or other people at risk.
      </p>

      <h2>Disclaimer and liability</h2>
      <p>
        {site.name} is provided as it is, without warranties beyond those the
        law requires. To the extent the law allows, <Company /> is not liable
        for indirect or consequential loss, and our total liability is limited
        to what you paid us in the 12 months before the claim.
      </p>
      <p>
        Nothing here removes rights you have as a consumer that cannot be
        removed by agreement.
      </p>

      <h2>Changes</h2>
      <p>
        We may update these terms. The date at the top shows the current
        version, and we will tell you in the app about a significant change.
        Continuing to use {site.name} after that means you accept the new
        terms.
      </p>

      <h2>Governing law</h2>
      <p>
        These terms are governed by the laws of{" "}
        {site.governingLaw || "[your jurisdiction]"}, without regard to
        conflict of law rules.
      </p>

      <h2>Contact</h2>
      <p>
        <Company />
        {site.address ? <>, {site.address}</> : null}
        {site.supportEmail ? (
          <>
            . Email <a href={`mailto:${site.supportEmail}`}>{site.supportEmail}</a>.
          </>
        ) : null}
      </p>
    </LegalPage>
  );
}
