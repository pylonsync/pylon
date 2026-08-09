import type { AuthInfo } from "./types";

export type AuthClaims = Omit<AuthInfo, "elevate">;

/** Normalize the Rust wire envelope into the public function auth shape. */
export function normalizeAuthClaims(
  raw: Record<string, unknown>,
): AuthClaims {
  return {
    userId:
      ((raw.userId ?? raw.user_id) as string | null | undefined) ?? null,
    isAdmin: Boolean(raw.isAdmin ?? raw.is_admin),
    tenantId:
      ((raw.tenantId ?? raw.tenant_id) as string | null | undefined) ?? null,
    roles: Array.isArray(raw.roles)
      ? raw.roles.filter((role): role is string => typeof role === "string")
      : [],
  };
}
