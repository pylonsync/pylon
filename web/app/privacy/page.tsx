import type { Metadata } from "@pylonsync/react";
import { LegalPage, LegalSection, LegalList } from "@pylon-cloud/ui/components/legal-page";

// Two cached shells keyed on the signed-in bit — see app/page.tsx for the
// full auth-bucketed rationale.
export const cache = "auth-bucketed";
export const revalidate = 3600; // 1 hour

export const metadata: Metadata = {
	title: "Privacy Policy — Pylon",
	description:
		"How Pylon collects, uses, shares, and protects your information when you use pylonsync.com and Pylon Cloud.",
	canonical: "/privacy",
	robots: "index,follow",
	openGraph: {
		title: "Privacy Policy — Pylon",
		description: "How Pylon handles your data.",
		url: "https://www.pylonsync.com/privacy",
		type: "article",
	},
};

export default function PrivacyPage({
	session,
}: {
	session?: { exists: boolean };
}) {
	return (
		<LegalPage
			signedIn={Boolean(session?.exists)}
			title="Privacy Policy"
			lastUpdated="July 8, 2026"
			intro={
				'This Privacy Policy explains how Pylon ("Pylon", "we", "us") collects, uses, discloses, and protects information when you visit pylonsync.com or use Pylon Cloud (together, the "Service"). By using the Service you agree to this Policy.'
			}
		>
			<LegalSection heading="1. Information we collect">
				<p>We collect the following categories of information:</p>
				<LegalList
					items={[
						<>
							<strong>Account information</strong> — your name, email address, a
							hashed password, and organization/workspace details you provide
							when you sign up (or the profile your OAuth provider shares if you
							sign in with Google or GitHub).
						</>,
						<>
							<strong>Billing information</strong> — plan, subscription status,
							and a customer identifier. Payments are processed by our payment
							provider (Stripe); we do not receive or store full payment-card
							numbers.
						</>,
						<>
							<strong>Content and application data</strong> — the projects,
							code, configuration, and data you create, deploy, or store through
							the Service.
						</>,
						<>
							<strong>Usage and diagnostic data</strong> — logs, metrics,
							feature usage, and error reports we generate to operate and improve
							the Service.
						</>,
						<>
							<strong>Technical data</strong> — IP address, browser and device
							information, and cookies or similar technologies used for
							authentication and preferences.
						</>,
					]}
				/>
			</LegalSection>

			<LegalSection heading="2. How we use information">
				<p>We use information to:</p>
				<LegalList
					items={[
						"Provide, operate, maintain, and secure the Service;",
						"Authenticate you and manage your account and workspaces;",
						"Process payments and manage subscriptions;",
						"Provide customer support and respond to your requests;",
						"Send service, security, and administrative communications;",
						"Monitor, debug, and improve the Service and develop new features;",
						"Detect, prevent, and address fraud, abuse, and security issues; and",
						"Comply with legal obligations and enforce our agreements.",
					]}
				/>
				<p>
					We do not sell your personal information, and we do not use your
					application content to train machine-learning models.
				</p>
			</LegalSection>

			<LegalSection heading="3. Legal bases (EEA/UK users)">
				<p>
					Where the GDPR or UK GDPR applies, we process personal data on the
					bases of <strong>performance of a contract</strong> (to provide the
					Service you request), <strong>legitimate interests</strong> (to
					secure, operate, and improve the Service),{" "}
					<strong>consent</strong> (where you provide it, such as optional
					communications), and <strong>legal obligation</strong>.
				</p>
			</LegalSection>

			<LegalSection heading="4. Your data vs. our role">
				<p>
					For personal data contained in the application data you store or
					process through the Service, <strong>you are the controller</strong>{" "}
					and Pylon acts as a <strong>processor</strong> on your behalf,
					handling that data according to your instructions and this Policy. You
					are responsible for having a lawful basis to collect and process that
					data and for informing your own end users.
				</p>
			</LegalSection>

			<LegalSection heading="5. Cookies">
				<p>
					We use strictly necessary cookies to keep you signed in and to
					remember your preferences. We do not use third-party advertising or
					cross-site tracking cookies. You can control cookies through your
					browser, though disabling essential cookies may prevent you from
					signing in.
				</p>
			</LegalSection>

			<LegalSection heading="6. How we share information">
				<p>We share information only in these circumstances:</p>
				<LegalList
					items={[
						<>
							<strong>Service providers (subprocessors)</strong> — trusted
							vendors that help us run the Service, such as cloud
							infrastructure/hosting, payment processing (Stripe), email
							delivery, and error monitoring. They may access data only to
							perform services for us and are bound by confidentiality
							obligations.
						</>,
						<>
							<strong>Legal and safety</strong> — when required by law, legal
							process, or to protect the rights, property, or safety of Pylon,
							our users, or the public.
						</>,
						<>
							<strong>Business transfers</strong> — in connection with a merger,
							acquisition, or sale of assets, subject to this Policy.
						</>,
					]}
				/>
				<p>We do not sell or rent your personal information.</p>
			</LegalSection>

			<LegalSection heading="7. Data retention">
				<p>
					We retain personal data for as long as your account is active and as
					needed to provide the Service, then for a reasonable period afterward
					to comply with legal, tax, accounting, or dispute-resolution
					obligations. When data is no longer needed, we delete or anonymize it.
					You can delete your workspace data at any time from the dashboard.
				</p>
			</LegalSection>

			<LegalSection heading="8. Security">
				<p>
					We use administrative, technical, and organizational safeguards to
					protect information, including encryption in transit, access controls,
					and least-privilege practices. No method of transmission or storage is
					100% secure, so we cannot guarantee absolute security.
				</p>
			</LegalSection>

			<LegalSection heading="9. International transfers">
				<p>
					We are based in the United States and may process and store
					information in the United States and other countries where we or our
					service providers operate. Where required, we rely on appropriate
					safeguards (such as Standard Contractual Clauses) for cross-border
					transfers.
				</p>
			</LegalSection>

			<LegalSection heading="10. Your rights">
				<p>
					Depending on where you live, you may have the right to access,
					correct, delete, port, or restrict processing of your personal data,
					and to object to certain processing or withdraw consent. If you are in
					California, you may request to know, delete, or correct your personal
					information and to opt out of its "sale" or "sharing" — which we do
					not do. To exercise any of these rights, contact us at the address
					below; we will not discriminate against you for doing so.
				</p>
			</LegalSection>

			<LegalSection heading="11. Children">
				<p>
					The Service is not directed to children under 16, and we do not
					knowingly collect personal information from them. If you believe a
					child has provided us personal information, contact us and we will
					delete it.
				</p>
			</LegalSection>

			<LegalSection heading="12. Changes to this Policy">
				<p>
					We may update this Policy from time to time. We will post the updated
					version here and revise the "Last updated" date, and for material
					changes we will provide additional notice (such as by email or an
					in-product notice).
				</p>
			</LegalSection>

			<LegalSection heading="13. Contact us">
				<p>
					Questions about this Policy or your data? Email us at{" "}
					<a href="mailto:privacy@pylonsync.com">privacy@pylonsync.com</a>.
				</p>
			</LegalSection>
		</LegalPage>
	);
}
