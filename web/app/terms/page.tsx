import type { Metadata } from "@pylonsync/react";
import { LegalPage, LegalSection, LegalList } from "@pylon-cloud/ui/components/legal-page";

// Two cached shells keyed on the signed-in bit — see app/page.tsx for the
// full auth-bucketed rationale.
export const cache = "auth-bucketed";
export const revalidate = 3600; // 1 hour
export const metadata: Metadata = {
	title: "Terms of Service — Pylon",
	description:
		"The terms that govern your access to and use of pylonsync.com and Stack0 Cloud.",
	canonical: "/terms",
	robots: "index,follow",
	openGraph: {
		title: "Terms of Service — Pylon",
		description: "The terms governing Stack0 Cloud.",
		url: "https://www.pylonsync.com/terms",
		type: "article",
	},
};

export default function TermsPage({
	session,
}: {
	session?: { exists: boolean };
}) {
	return (
		<LegalPage
			signedIn={Boolean(session?.exists)}
			title="Terms of Service"
			lastUpdated="July 8, 2026"
			intro={
				'These Terms of Service ("Terms") govern your access to and use of pylonsync.com and the Stack0 Cloud hosting service (the "Service"), operated by Pylon. By creating an account or using the Service, you agree to these Terms. If you use the Service on behalf of an organization, you represent that you have authority to bind it.'
			}
		>
			<LegalSection heading="1. The Service">
				<p>
					Stack0 Cloud is a hosted platform for building, deploying, and running
					applications on the Pylon framework. We may add, change, or remove
					features over time. The open-source Pylon software is a separate
					offering licensed under its own terms (MIT or Apache-2.0); these Terms
					govern the hosted Service only.
				</p>
			</LegalSection>

			<LegalSection heading="2. Accounts">
				<p>
					You must provide accurate information, keep your credentials secure,
					and are responsible for all activity under your account. You must be
					at least the age of majority in your jurisdiction. Notify us promptly
					of any unauthorized use.
				</p>
			</LegalSection>

			<LegalSection heading="3. Acceptable use">
				<p>You agree not to use the Service to:</p>
				<LegalList
					items={[
						"Violate any law or regulation, or infringe others' rights;",
						"Store or transmit malware, or launch attacks against any system;",
						"Send spam or unsolicited communications, or harvest data unlawfully;",
						"Attempt to gain unauthorized access to the Service or other users' data;",
						"Interfere with, disrupt, or place undue load on the Service;",
						"Resell or provide the Service to third parties except as permitted; or",
						"Use the Service for high-risk activities where failure could lead to death, injury, or environmental harm.",
					]}
				/>
				<p>
					We may suspend or limit accounts that violate these Terms or threaten
					the security or integrity of the Service.
				</p>
			</LegalSection>

			<LegalSection heading="4. Your content and data">
				<p>
					You retain all ownership of the code, content, and data you submit to
					or create through the Service ("Your Content"). You grant Pylon a
					limited, worldwide, non-exclusive license to host, copy, process,
					transmit, and display Your Content solely as necessary to provide and
					support the Service. You are responsible for Your Content, for having
					the rights to use it, and for your own end users.
				</p>
			</LegalSection>

			<LegalSection heading="5. Intellectual property">
				<p>
					Pylon and its licensors own the Service, our software, and our
					trademarks and branding. Except for the rights expressly granted here,
					no rights are transferred to you. If you send us feedback or
					suggestions, you grant us a perpetual, royalty-free license to use them
					without obligation to you.
				</p>
			</LegalSection>

			<LegalSection heading="6. Fees and billing">
				<p>
					Paid plans are billed in advance on a recurring basis (monthly or
					annually) through our payment processor, and renew automatically until
					canceled. Fees are exclusive of taxes, which you are responsible for.
					Except where required by law or expressly stated, payments are
					non-refundable, including for partial billing periods. We may change
					pricing with reasonable prior notice; changes take effect on your next
					billing cycle. You can cancel at any time, effective at the end of the
					current period.
				</p>
			</LegalSection>

			<LegalSection heading="7. Third-party services and beta features">
				<p>
					The Service may integrate with third-party services governed by their
					own terms; we are not responsible for them. Features labeled beta,
					preview, or experimental are provided "as is," may change or be
					discontinued, and are excluded from any service commitments.
				</p>
			</LegalSection>

			<LegalSection heading="8. Availability and disclaimers">
				<p>
					The Service is provided <strong>"as is" and "as available,"</strong>{" "}
					without warranties of any kind, whether express, implied, or
					statutory, including warranties of merchantability, fitness for a
					particular purpose, and non-infringement. We do not warrant that the
					Service will be uninterrupted, error-free, or secure, or that data
					will not be lost. You are responsible for maintaining your own backups
					of Your Content; export tools are available.
				</p>
			</LegalSection>

			<LegalSection heading="9. Limitation of liability">
				<p>
					To the maximum extent permitted by law, Pylon will not be liable for
					any indirect, incidental, special, consequential, or punitive damages,
					or for lost profits, revenue, data, or goodwill. Our total liability
					arising out of or relating to the Service will not exceed the greater
					of (a) the amounts you paid us for the Service in the twelve months
					before the event giving rise to the claim, or (b) US $100.
				</p>
			</LegalSection>

			<LegalSection heading="10. Indemnification">
				<p>
					You will defend, indemnify, and hold harmless Pylon from claims,
					damages, and expenses arising out of Your Content, your use of the
					Service, or your violation of these Terms or applicable law.
				</p>
			</LegalSection>

			<LegalSection heading="11. Termination">
				<p>
					You may stop using the Service and delete your account at any time.
					We may suspend or terminate access if you materially breach these
					Terms, fail to pay, or use the Service in a way that risks harm to
					others. After termination we will make Your Content available for
					export for a reasonable period, then delete it in the ordinary course.
					Sections that by their nature should survive (including ownership,
					disclaimers, liability limits, and indemnification) survive
					termination.
				</p>
			</LegalSection>

			<LegalSection heading="12. Governing law and disputes">
				<p>
					These Terms are governed by the laws of the State of Texas, USA,
					without regard to its conflict-of-laws rules. You and Pylon agree to
					the exclusive jurisdiction of the state and federal courts located in
					Dallas County, Texas, and will first attempt to resolve any dispute
					informally by contacting us.
				</p>
			</LegalSection>

			<LegalSection heading="13. Changes to these Terms">
				<p>
					We may update these Terms from time to time. We will post the updated
					version here and revise the "Last updated" date, and for material
					changes we will provide additional notice. Your continued use of the
					Service after changes take effect constitutes acceptance.
				</p>
			</LegalSection>

			<LegalSection heading="14. Contact us">
				<p>
					Questions about these Terms? Email us at{" "}
					<a href="mailto:legal@pylonsync.com">legal@pylonsync.com</a>.
				</p>
			</LegalSection>
		</LegalPage>
	);
}
