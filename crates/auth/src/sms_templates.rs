//! Text of the sign-in code SMS (`/api/auth/phone/send-code`).
//!
//! `PYLON_SMS_TEMPLATE_SIGN_IN_CODE` overrides the text. It may use
//! `{{code}}` and `{{app_name}}`; any other `{{...}}` is dropped, as in
//! [`crate::email_templates`]. The app name comes from `PYLON_APP_NAME`,
//! then `PYLON_EMAIL_APP_NAME`.
//!
//! Carriers reviewing an A2P 10DLC campaign expect the text to name the
//! sender, so the default includes the app name when one is set.

use std::collections::HashMap;

/// Env var that overrides the sign-in code text.
pub const SIGN_IN_CODE_ENV: &str = "PYLON_SMS_TEMPLATE_SIGN_IN_CODE";

/// Default text when an app name is set.
pub const DEFAULT_SIGN_IN_CODE_WITH_APP: &str =
    "{{app_name}}: your sign-in code is {{code}}. It expires in 10 minutes.";

/// Default text when no app name is set.
pub const DEFAULT_SIGN_IN_CODE: &str = "Your sign-in code is {{code}}. It expires in 10 minutes.";

const ALLOWED_VARS: &[&str] = &["code", "app_name"];

/// The app name for SMS: `PYLON_APP_NAME`, else `PYLON_EMAIL_APP_NAME`,
/// else empty.
pub fn app_name() -> String {
    ["PYLON_APP_NAME", "PYLON_EMAIL_APP_NAME"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .map(|v| v.trim().to_string())
        .find(|v| !v.is_empty())
        .unwrap_or_default()
}

/// The sign-in code text, from the env override or the default.
pub fn render_sign_in_code(code: &str) -> String {
    let template = std::env::var(SIGN_IN_CODE_ENV).ok();
    render_sign_in_code_with(template.as_deref(), &app_name(), code)
}

/// [`render_sign_in_code`] with the template and app name passed in.
/// An empty or blank `template` means the default.
pub fn render_sign_in_code_with(template: Option<&str>, app_name: &str, code: &str) -> String {
    let template = match template.filter(|t| !t.trim().is_empty()) {
        Some(t) => t,
        None if app_name.is_empty() => DEFAULT_SIGN_IN_CODE,
        None => DEFAULT_SIGN_IN_CODE_WITH_APP,
    };
    let mut vars = HashMap::new();
    vars.insert("code", code);
    vars.insert("app_name", app_name);
    crate::email_templates::substitute(template, ALLOWED_VARS, &vars)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_names_the_app_when_set() {
        assert_eq!(
            render_sign_in_code_with(None, "PEMF Recovery Plus", "0427"),
            "PEMF Recovery Plus: your sign-in code is 0427. It expires in 10 minutes."
        );
    }

    #[test]
    fn default_without_app_name() {
        assert_eq!(
            render_sign_in_code_with(None, "", "123456"),
            "Your sign-in code is 123456. It expires in 10 minutes."
        );
        assert_eq!(
            render_sign_in_code_with(Some("  "), "", "123456"),
            "Your sign-in code is 123456. It expires in 10 minutes."
        );
    }

    #[test]
    fn override_substitutes_allowed_vars_only() {
        assert_eq!(
            render_sign_in_code_with(
                Some("{{app_name}} code {{code}}{{secret}}. Reply STOP to opt out."),
                "Acme",
                "9081"
            ),
            "Acme code 9081. Reply STOP to opt out."
        );
    }
}
