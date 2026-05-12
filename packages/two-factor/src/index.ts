/**
 * `@pylonsync/two-factor` — TOTP + backup codes for Pylon apps.
 *
 * Three primitives:
 *   - `generateSecret()` — for TOTP enrollment
 *   - `verifyTotp(secret, code, { lastAcceptedCounter })` — verify
 *     with skew tolerance + replay protection (rejects same-window
 *     re-use, per RFC 6238 §5.2)
 *   - `generateBackupCodes()` + `hashBackupCode()` + `verifyBackupCode()`
 *
 * Plus an `otpAuthUri()` builder for QR code rendering on the
 * enrollment screen.
 *
 * Schema additions (declared on app side):
 *   - `User.twoFactorEnabled: boolean`
 *   - `TwoFactor { userId, secret (encrypted), backupCodes (encrypted),
 *                  lastAcceptedCounter, createdAt }`
 *
 * Secret storage:
 *   - The plugin does NOT encrypt the secret for you. Apps are
 *     expected to wrap secret storage with their own at-rest
 *     encryption (Pylon's `crypto/secrets` provider, KMS, etc.) —
 *     because key management is app-specific (some apps have a
 *     master key, others use per-user keys, others use HSMs).
 *     We surface the primitive, not the policy.
 *
 * Trusted-device cookie:
 *   - 30-day session cookie scoped per device, set after a
 *     successful verifyTotp when the user opted in. Pylon's session
 *     cookie helper (`@pylonsync/sdk/cookie`) handles the actual
 *     Set-Cookie emission; this package only generates the random
 *     device token + provides `isTrustedDevice(deviceToken)` to
 *     skip TOTP for the trusted window.
 */

export {
	generateSecret,
	otpAuthUri,
	generateTotp,
	verifyTotp,
	type TotpConfig,
} from "./totp";

export {
	generateBackupCodes,
	hashBackupCode,
	verifyBackupCode,
	type BackupCodeConfig,
} from "./backup-codes";

/**
 * Generate a trusted-device token. Apps store this hashed on the
 * `TwoFactor.trustedDevices` collection + set the raw value as an
 * HTTP-only cookie scoped to the app's hostname with a 30-day Max-Age.
 * On subsequent logins, look up the cookie's hash; if present + not
 * expired, skip the TOTP challenge.
 */
export function generateTrustedDeviceToken(): string {
	const bytes = new Uint8Array(32);
	crypto.getRandomValues(bytes);
	return [...bytes].map((b) => b.toString(16).padStart(2, "0")).join("");
}

/**
 * Hash a trusted-device token for storage. Same cheap-hash strategy
 * as backup codes — entropy comes from generation, so we just need
 * a one-way function to defeat lookups by raw value if the DB is
 * exfiltrated.
 */
export async function hashTrustedDeviceToken(token: string): Promise<string> {
	const data = new TextEncoder().encode(token);
	const buf = new ArrayBuffer(data.byteLength);
	new Uint8Array(buf).set(data);
	const hash = await crypto.subtle.digest("SHA-256", buf);
	return [...new Uint8Array(hash)]
		.map((b) => b.toString(16).padStart(2, "0"))
		.join("");
}
