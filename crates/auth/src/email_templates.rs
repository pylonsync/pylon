//! Email template customization with safe variable substitution.
//!
//! Pylon's auth flows send 5 transactional emails: magic code,
//! magic link, password reset, email change confirmation, org
//! invite. Apps that want branded copy override the subject + body
//! via env vars (`PYLON_EMAIL_TEMPLATE_<KIND>_{SUBJECT,BODY}`).
//!
//! **Security posture:**
//! - `{{var}}` substitution ONLY for the per-template allowlisted
//!   variable names. Anything else in the template is literal —
//!   no `{{ env.SECRET }}` or `{{ exec("rm -rf /") }}` injection
//!   vectors, no Tera/Handlebars surface.
//! - Templates come from env vars set by the operator (not user
//!   input), so SSTI risk is operator-only — but defense in depth:
//!   we still constrain what `{{...}}` can resolve to.
//! - Each variable's value comes from server-controlled data
//!   (token URL, app name) — caller is responsible for not
//!   stuffing user input into a template var without escaping.

use std::collections::HashMap;

/// The five built-in transactional emails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EmailTemplate {
    /// 6-digit magic code (`/api/auth/magic/send`). Vars: `{{code}}`.
    MagicCode,
    /// One-shot magic link (`/api/auth/magic-link/send`).
    /// Vars: `{{url}}`.
    MagicLink,
    /// Password reset link (`/api/auth/password/reset/request`).
    /// Vars: `{{url}}`.
    PasswordReset,
    /// Email-change confirmation link sent to NEW address
    /// (`/api/auth/email/change/request`). Vars: `{{url}}`.
    EmailChangeConfirm,
    /// Org invite (`/api/auth/orgs/:id/invites`). Vars:
    /// `{{org_name}}`, `{{url}}`.
    OrgInvite,
}

impl EmailTemplate {
    /// Stable string for the env-var prefix.
    pub fn env_key(&self) -> &'static str {
        match self {
            Self::MagicCode => "MAGIC_CODE",
            Self::MagicLink => "MAGIC_LINK",
            Self::PasswordReset => "PASSWORD_RESET",
            Self::EmailChangeConfirm => "EMAIL_CHANGE",
            Self::OrgInvite => "ORG_INVITE",
        }
    }

    /// Default subject. Plain English; override via
    /// `PYLON_EMAIL_TEMPLATE_<KIND>_SUBJECT`.
    pub fn default_subject(&self) -> &'static str {
        match self {
            Self::MagicCode => "Your sign-in code",
            Self::MagicLink => "Sign in to your account",
            Self::PasswordReset => "Reset your password",
            Self::EmailChangeConfirm => "Confirm your email change",
            Self::OrgInvite => "You've been invited to {{org_name}}",
        }
    }

    /// Default body. Override via `PYLON_EMAIL_TEMPLATE_<KIND>_BODY`.
    /// Plain text — the email transport doesn't render HTML
    /// (apps that want HTML emails should configure their SMTP
    /// transport accordingly + override these templates).
    pub fn default_body(&self) -> &'static str {
        match self {
            Self::MagicCode => {
                "Your sign-in code is: {{code}}\n\nThis code will expire in 10 minutes."
            }
            Self::MagicLink => {
                "Click here to sign in:\n\n{{url}}\n\nThis link expires in 15 minutes. \
                 If you didn't request it, ignore this email."
            }
            Self::PasswordReset => {
                "Reset your password by visiting:\n\n{{url}}\n\nThis link expires in 30 \
                 minutes. If you didn't request a reset, ignore this email."
            }
            Self::EmailChangeConfirm => {
                "Confirm your new email by visiting:\n\n{{url}}\n\nThis link expires in 24 \
                 hours. If you didn't request this change, ignore the email."
            }
            Self::OrgInvite => {
                "You've been invited to join {{org_name}} on Pylon.\n\nAccept here: \
                 {{url}}\n\nThis link expires in 7 days."
            }
        }
    }

    /// Variable names the template may reference. Substitution
    /// silently drops `{{x}}` for any `x` not in this list (NOT
    /// "leave it raw" — that would surface internal placeholders
    /// to end users on misconfigured templates).
    pub fn allowed_vars(&self) -> &'static [&'static str] {
        match self {
            Self::MagicCode => &["code"],
            Self::MagicLink => &["url"],
            Self::PasswordReset => &["url"],
            Self::EmailChangeConfirm => &["url"],
            Self::OrgInvite => &["org_name", "url"],
        }
    }
}

/// Render a template into `(subject, body)`. Pulls env overrides
/// when `PYLON_EMAIL_TEMPLATE_<KIND>_{SUBJECT,BODY}` are set;
/// falls back to the bundled defaults otherwise.
pub fn render(template: EmailTemplate, vars: &HashMap<&str, &str>) -> (String, String) {
    let subject_raw = std::env::var(format!(
        "PYLON_EMAIL_TEMPLATE_{}_SUBJECT",
        template.env_key()
    ))
    .ok()
    .filter(|s| !s.is_empty())
    .unwrap_or_else(|| template.default_subject().to_string());
    let body_raw = std::env::var(format!(
        "PYLON_EMAIL_TEMPLATE_{}_BODY",
        template.env_key()
    ))
    .ok()
    .filter(|s| !s.is_empty())
    .unwrap_or_else(|| template.default_body().to_string());
    (
        substitute(&subject_raw, template.allowed_vars(), vars),
        substitute(&body_raw, template.allowed_vars(), vars),
    )
}

/// `{{var}}` substitution restricted to `allowed`. Unknown vars are
/// dropped silently (the placeholder is removed) — this prevents
/// raw `{{secret_key}}` from showing up in user emails when the
/// operator typos a template.
///
/// Only `{{name}}` (alphanumeric + underscore) is recognized. No
/// expressions, no method calls, no format specifiers, no nested
/// braces. Anything that doesn't match the simple shape is left
/// literal in the output.
fn substitute(template: &str, allowed: &[&str], vars: &HashMap<&str, &str>) -> String {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Look for `{{`
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            // Find the closing `}}`. Cap the scan to defeat a
            // pathological `{{<10MB of text>` input — if we don't
            // find `}}` within a generous window, treat the `{{`
            // as literal.
            const MAX_VAR_LEN: usize = 64;
            let body_start = i + 2;
            let scan_end = (body_start + MAX_VAR_LEN).min(bytes.len());
            let mut closing: Option<usize> = None;
            let mut j = body_start;
            while j + 1 < scan_end {
                if bytes[j] == b'}' && bytes[j + 1] == b'}' {
                    closing = Some(j);
                    break;
                }
                j += 1;
            }
            match closing {
                Some(end) => {
                    let var_name = &template[body_start..end];
                    // Allowlist + alphanumeric/underscore name check.
                    let valid_name = !var_name.is_empty()
                        && var_name
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'_');
                    if valid_name && allowed.contains(&var_name) {
                        if let Some(value) = vars.get(var_name) {
                            out.push_str(value);
                        }
                        // Unknown-but-allowed: drop silently.
                    } else {
                        // Disallowed name: drop the placeholder.
                        // (Do NOT echo `{{x}}` literally — that would
                        // leak internal placeholder names to end users.)
                    }
                    i = end + 2;
                    continue;
                }
                None => {
                    // No closing — treat the `{{` as literal so we
                    // don't swallow the rest of the template.
                    out.push('{');
                    out.push('{');
                    i += 2;
                    continue;
                }
            }
        }
        // Push one full UTF-8 char.
        let (_, ch) = template[i..].char_indices().next().expect("non-empty");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn default_substitution() {
        let mut vars = HashMap::new();
        vars.insert("code", "123456");
        let (subject, body) = render(EmailTemplate::MagicCode, &vars);
        assert_eq!(subject, "Your sign-in code");
        assert!(body.contains("Your sign-in code is: 123456"));
    }

    #[test]
    fn env_override_replaces_subject_and_body() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::set_var("PYLON_EMAIL_TEMPLATE_MAGIC_CODE_SUBJECT", "Acme code");
        std::env::set_var(
            "PYLON_EMAIL_TEMPLATE_MAGIC_CODE_BODY",
            "Your Acme code: {{code}}",
        );
        let mut vars = HashMap::new();
        vars.insert("code", "999000");
        let (subject, body) = render(EmailTemplate::MagicCode, &vars);
        assert_eq!(subject, "Acme code");
        assert_eq!(body, "Your Acme code: 999000");
        std::env::remove_var("PYLON_EMAIL_TEMPLATE_MAGIC_CODE_SUBJECT");
        std::env::remove_var("PYLON_EMAIL_TEMPLATE_MAGIC_CODE_BODY");
    }

    #[test]
    fn unknown_var_silently_dropped() {
        // The default magic-code template uses `{{code}}`, NOT
        // `{{secret_key}}`. If a typo'd override references a
        // disallowed name, the placeholder is removed (NOT echoed
        // literally to the user).
        let template = "Hello {{secret_key}} world";
        let allowed = &["code"];
        let vars = HashMap::new();
        assert_eq!(substitute(template, allowed, &vars), "Hello  world");
    }

    #[test]
    fn allowed_var_with_no_value_silently_dropped() {
        let template = "Code: {{code}}";
        let allowed = &["code"];
        let vars = HashMap::new(); // no `code` key
        assert_eq!(substitute(template, allowed, &vars), "Code: ");
    }

    #[test]
    fn malformed_placeholder_treated_literally() {
        // `{{` with no closing → keep both braces literally.
        let allowed = &["code"];
        let vars = HashMap::new();
        assert_eq!(substitute("price {{ 50%", allowed, &vars), "price {{ 50%");
    }

    #[test]
    fn cannot_inject_special_template_syntax() {
        // Things like `{{ exec() }}` or `{{ env.SECRET }}` are NOT
        // valid `{{name}}` shapes. The allowlist + name validator
        // rejects them and the placeholder is dropped silently.
        // Spaces are also disallowed in our minimal subst grammar
        // — `{{ code }}` won't expand to the value of `code`.
        let allowed = &["code"];
        let mut vars = HashMap::new();
        vars.insert("code", "x");
        // Result is "a  b  c" — the `{{exec()}}` and `{{env.SECRET}}`
        // tokens are dropped, leaving just spaces.
        assert_eq!(
            substitute("a {{exec()}} b {{env.SECRET}} c", allowed, &vars),
            "a  b  c"
        );
        // And literal `code` reference outside the brace shape isn't
        // expanded either.
        assert_eq!(substitute("code = $code", allowed, &vars), "code = $code");
    }

    #[test]
    fn multibyte_input_does_not_panic() {
        let allowed = &["code"];
        let mut vars = HashMap::new();
        vars.insert("code", "✨");
        let out = substitute("Hello 🌍! Code: {{code}}", allowed, &vars);
        assert_eq!(out, "Hello 🌍! Code: ✨");
    }

    #[test]
    fn long_garbage_inside_placeholder_treated_literally() {
        // 10k chars between `{{` and `}}` — must not OOM or
        // hang. The MAX_VAR_LEN cap kicks in.
        let allowed = &["code"];
        let vars = HashMap::new();
        let evil = format!("{{{{{}}}}}", "a".repeat(10_000));
        let out = substitute(&evil, allowed, &vars);
        // Either dropped (if the cap finds a `}}` somehow) or
        // left literal. Both are acceptable; the function must
        // just terminate quickly.
        assert!(out.len() <= evil.len() + 4);
    }

    #[test]
    fn allowed_vars_per_template_are_distinct() {
        // Sanity: org-invite supports `{{org_name}}` AND `{{url}}`.
        let mut vars = HashMap::new();
        vars.insert("org_name", "Acme");
        vars.insert("url", "https://x/accept");
        let (subject, body) = render(EmailTemplate::OrgInvite, &vars);
        assert!(subject.contains("Acme"));
        assert!(body.contains("Acme"));
        assert!(body.contains("https://x/accept"));
    }

    #[test]
    fn empty_env_value_falls_back_to_default() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::set_var("PYLON_EMAIL_TEMPLATE_MAGIC_LINK_SUBJECT", "");
        let vars = HashMap::new();
        let (subject, _) = render(EmailTemplate::MagicLink, &vars);
        // Empty env value treated as unset.
        assert_eq!(subject, "Sign in to your account");
        std::env::remove_var("PYLON_EMAIL_TEMPLATE_MAGIC_LINK_SUBJECT");
    }
}
