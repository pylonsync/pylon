import React from "react";
import { type Metadata } from "@pylonsync/react";
import { Company, LegalPage } from "@/components/legal";
import { site } from "@/lib/site";

export const metadata: Metadata = {
  title: `Privacy Policy — ${site.name}`,
  description: `How ${site.name} handles your data.`,
};

/**
 * Both stores require a reachable privacy policy URL before they will accept
 * a build, and Apple checks that it matches the App Privacy answers you give
 * in App Store Connect. This text describes what the template's app really
 * does: accounts, notes, and store subscriptions, with no advertising or
 * tracking. Change it as your app changes.
 */
export default function PrivacyPage() {
  return (
    <LegalPage title="Privacy Policy">
      <p>
        {site.name} is published by <Company />. This page explains what we
        collect, why we have it, and how to get rid of it.
      </p>

      <h2>What we collect</h2>
      <ul>
        <li>
          <strong>Your account.</strong> An email address when you sign in with
          a code, or the account identifier and email that Apple or Google
          gives us when you use their sign-in. Apple lets you hide your real
          address, and that works here.
        </li>
        <li>
          <strong>What you write.</strong> The notes you create, so we can
          store them and sync them to your other devices.
        </li>
        <li>
          <strong>Your subscription status.</strong> Whether a subscription is
          active, and which product it is. Apple and Google take the payment.
          We never receive your card details.
        </li>
        <li>
          <strong>Technical records.</strong> Ordinary server logs, including
          the internet address your device connects from and the time of the
          request. We use them to keep the service running and to spot abuse.
        </li>
      </ul>
      <p>
        You can use the app without an account. In that case the app still
        creates an anonymous identifier so your notes can sync to the server
        and back. Sign in later and that content moves to your account.
      </p>

      <h2>What we do not do</h2>
      <ul>
        <li>We do not sell your data.</li>
        <li>
          We do not use it for advertising, and we do not track you across
          other apps or websites.
        </li>
        <li>We do not read your notes, except when you ask us to help.</li>
      </ul>

      <h2>Who else touches it</h2>
      <p>
        We use a small number of companies to run the service, and they may
        process data on our behalf:
      </p>
      <ul>
        <li>Apple and Google, for sign-in, payments, and app distribution.</li>
        <li>RevenueCat, to tell us whether a subscription is active.</li>
        <li>Our hosting provider, which stores the data and serves the API.</li>
        <li>Our email provider, to deliver sign-in codes.</li>
      </ul>

      <h2>How long we keep it</h2>
      <p>
        Your notes stay until you delete them or delete your account. Logs are
        kept for a short period and then discarded.
      </p>

      <h2>Deleting your account</h2>
      <p>
        Open the app and go to Settings, then Delete account. This removes your
        account and the notes attached to it. It happens immediately and it
        cannot be undone. A subscription is billed by the store, so cancel it
        in your App Store or Play Store account as well.
      </p>

      <h2>Your rights</h2>
      <p>
        You can ask what we hold about you, ask for a copy, ask us to correct
        it, or ask us to delete it. Write to us at the address below. Depending
        on where you live, you may have further rights under local law.
      </p>

      <h2>Children</h2>
      <p>
        {site.name} is not directed at children under 13, and we do not
        knowingly collect their data. If you believe a child has given us
        information, contact us and we will remove it.
      </p>

      <h2>Security</h2>
      <p>
        Traffic between the app and our servers is encrypted in transit. We
        limit who can reach production data. No service can promise perfect
        security, and we do not.
      </p>

      <h2>Changes</h2>
      <p>
        If this policy changes in a way that matters, we will update the date
        at the top of this page and, where the change is significant, tell you
        in the app.
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
