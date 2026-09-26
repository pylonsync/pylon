// iMessage handles: a phone number in E.164 form or an email address (Apple ID).
// Every handle that enters the database or leaves through a transport goes
// through `normalizeHandle`, so one person has exactly one Contact row no matter
// how a transport formats the number.

export type HandleKind = "phone" | "email";

export interface NormalizedHandle {
  handle: string;
  kind: HandleKind;
}

const EMAIL_RE = /^[a-z0-9._%+-]{1,64}@[a-z0-9.-]{1,253}\.[a-z]{2,24}$/;
const E164_RE = /^\+[1-9][0-9]{6,14}$/;

/**
 * Normalize a raw handle. Returns null for anything that is not a plausible
 * phone number or email, including group-chat identifiers (`chat123…`).
 *
 * Phone rules: strip spaces, dashes, dots and parentheses. A leading `+` keeps
 * the country code as written. Ten bare digits get `defaultCountryCode`
 * (default "1", North America). Eleven digits starting with that code get a `+`.
 */
export function normalizeHandle(
  raw: string | null | undefined,
  defaultCountryCode = "1",
): NormalizedHandle | null {
  if (typeof raw !== "string") return null;
  let value = raw.trim();
  if (value === "" || value.length > 320) return null;
  // Some transports prefix the service: "tel:+1555…", "mailto:a@b.c", "iMessage;-;+1555…".
  value = value.replace(/^(tel:|mailto:|sms:|imessage:)/i, "");
  const semi = value.lastIndexOf(";");
  if (semi !== -1) value = value.slice(semi + 1);

  if (value.includes("@")) {
    const email = value.toLowerCase();
    return EMAIL_RE.test(email) ? { handle: email, kind: "email" } : null;
  }

  const hasPlus = value.startsWith("+");
  if (!/^\+?[0-9\s().-]+$/.test(value)) return null;
  const digits = value.replace(/[^0-9]/g, "");
  let e164: string;
  if (hasPlus) {
    e164 = `+${digits}`;
  } else if (digits.length === 10) {
    e164 = `+${defaultCountryCode}${digits}`;
  } else if (digits.length === 11 && digits.startsWith(defaultCountryCode)) {
    e164 = `+${digits}`;
  } else {
    return null;
  }
  return E164_RE.test(e164) ? { handle: e164, kind: "phone" } : null;
}

/** True when `value` is already a normalized handle. */
export function isNormalizedHandle(value: string): boolean {
  return E164_RE.test(value) || EMAIL_RE.test(value);
}

/** "+15551234567" → "(555) 123-4567" for North American numbers; others unchanged. */
export function formatHandle(handle: string): string {
  const m = handle.match(/^\+1(\d{3})(\d{3})(\d{4})$/);
  if (m) return `(${m[1]}) ${m[2]}-${m[3]}`;
  return handle;
}
