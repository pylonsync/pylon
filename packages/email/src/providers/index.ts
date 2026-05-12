import { logProvider } from "./log";
import { postmarkProvider } from "./postmark";
import { resendProvider } from "./resend";
import type { EmailProvider, EmailProviderName } from "../types";

const REGISTRY: Record<EmailProviderName, EmailProvider> = {
	resend: resendProvider,
	postmark: postmarkProvider,
	log: logProvider,
	// SES + raw SMTP land next — they share a payload shape with
	// the providers above so adding them is a no-op for users.
	// Stubbed here so the typecheck stays exhaustive.
	ses: logProvider,
	smtp: logProvider,
};

export function getProvider(name: EmailProviderName): EmailProvider {
	const p = REGISTRY[name];
	if (!p) throw new Error(`unknown email provider: ${name}`);
	return p;
}
