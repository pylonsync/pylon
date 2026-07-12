// Pure project helpers. AGENTS.md's testing guidance: keep the decision logic
// out of the handler and in a pure function here, so it's exhaustively testable
// without a running server (see tests/projects.test.ts). functions/updateProject.ts
// is a thin wrapper around this.

/**
 * Trim + validate a project name. Returns the cleaned name, or null when it's
 * empty (or whitespace-only) or longer than 80 characters after trimming.
 */
export function normalizeProjectName(raw: string): string | null {
  const name = raw.trim();
  if (name.length < 1 || name.length > 80) return null;
  return name;
}
