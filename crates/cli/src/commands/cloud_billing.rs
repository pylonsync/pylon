//! `pylon billing` — show the caller's org plan, this month's usage charge +
//! projected month-end, and recent invoices.
//! `pylon billing upgrade` — open the Stripe checkout to move the org to Pro.
//!
//! Both resolve the org via `--org <slug>` or the single-org default (same rule
//! as `pylon projects`), so single-org users never think about orgs.

use pylon_kernel::ExitCode;
use serde::{Deserialize, Serialize};

use crate::cloud_client::{dashboard_url, post_json, require_credentials, Credentials};
use crate::output;

#[derive(Deserialize)]
struct OrgRow {
    id: String,
    slug: String,
    name: String,
}

#[derive(Serialize)]
struct OrgArg<'a> {
    #[serde(rename = "orgId")]
    org_id: &'a str,
}

/// `--org <slug>` → single-org default. Errors (with the org list) on
/// multi-org accounts that didn't pass `--org`.
fn resolve_org(args: &[String], creds: &Credentials) -> Result<OrgRow, String> {
    let wanted = args
        .windows(2)
        .find(|w| w[0] == "--org")
        .map(|w| w[1].clone())
        .or_else(|| {
            args.iter()
                .find(|a| a.starts_with("--org="))
                .map(|a| a.trim_start_matches("--org=").to_string())
        });
    let mut orgs: Vec<OrgRow> = post_json(creds, "/api/fn/listMyOrgsForCli", &())
        .map_err(|e| format!("Couldn't list your orgs: {e}"))?;
    if orgs.is_empty() {
        return Err(format!(
            "Your account has no organizations — finish signup at {}/dashboard.",
            dashboard_url()
        ));
    }
    match wanted {
        Some(slug) => orgs
            .into_iter()
            .find(|o| o.slug == slug)
            .ok_or_else(|| format!("You're not a member of an org with slug \"{slug}\".")),
        None if orgs.len() == 1 => Ok(orgs.remove(0)),
        None => {
            let list = orgs
                .iter()
                .map(|o| format!("    {} ({})", o.slug, o.name))
                .collect::<Vec<_>>()
                .join("\n");
            Err(format!(
                "You belong to multiple orgs — pass --org <slug>.\n{list}"
            ))
        }
    }
}

#[derive(Deserialize)]
struct FlyCost {
    plan: String,
    #[serde(rename = "currentChargeCents")]
    current_charge_cents: i64,
    #[serde(rename = "projectedChargeCents")]
    projected_charge_cents: i64,
}

#[derive(Deserialize)]
struct InvoicesResp {
    invoices: Vec<Invoice>,
}

#[derive(Deserialize)]
struct Invoice {
    #[serde(rename = "periodStart")]
    period_start: String,
    #[serde(rename = "periodEnd")]
    period_end: String,
    #[serde(rename = "amountCents")]
    amount_cents: i64,
    status: String,
}

fn plan_label(plan: &str) -> &str {
    match plan {
        "pro" => "Pro",
        "hobby" => "Hobby",
        "team" => "Team",
        "enterprise" => "Enterprise",
        other => other,
    }
}

fn dollars(cents: i64) -> String {
    format!("${:.2}", cents as f64 / 100.0)
}

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    // `pylon billing upgrade` is the plan change. It used to live at the
    // top level as `pylon upgrade`, which collided with what every other
    // CLI means by that word — `pylon upgrade` now updates the binary.
    if crate::commands::args::collect_positional(args, "billing").first() == Some(&"upgrade") {
        return run_upgrade(args, json_mode);
    }
    let creds = match require_credentials() {
        Ok(c) => c,
        Err(e) => {
            output::print_error(&e);
            eprintln!("  Run: pylon login");
            return ExitCode::Usage;
        }
    };
    let org = match resolve_org(args, &creds) {
        Ok(o) => o,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Usage;
        }
    };

    let cost: FlyCost = match post_json(
        &creds,
        "/api/fn/getOrgFlyCostThisMonth",
        &OrgArg { org_id: &org.id },
    ) {
        Ok(c) => c,
        Err(e) => {
            output::print_error(&format!("Couldn't load usage: {e}"));
            return ExitCode::Error;
        }
    };
    let invoices: Vec<Invoice> = post_json::<_, InvoicesResp>(
        &creds,
        "/api/fn/listOrgInvoices",
        &OrgArg { org_id: &org.id },
    )
    .map(|r| r.invoices)
    .unwrap_or_default();

    if json_mode {
        let out = serde_json::json!({
            "org": org.slug,
            "plan": cost.plan,
            "currentChargeCents": cost.current_charge_cents,
            "projectedChargeCents": cost.projected_charge_cents,
            "invoices": invoices.iter().map(|i| serde_json::json!({
                "periodStart": i.period_start,
                "periodEnd": i.period_end,
                "amountCents": i.amount_cents,
                "status": i.status,
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
        return ExitCode::Ok;
    }

    println!();
    println!("  Billing — {} ({})", org.slug, plan_label(&cost.plan));
    println!();
    println!("  This month");
    println!(
        "    Usage charge so far   {}",
        dollars(cost.current_charge_cents)
    );
    println!(
        "    Projected month-end   {}",
        dollars(cost.projected_charge_cents)
    );
    println!();
    if invoices.is_empty() {
        println!("  No invoices yet.");
    } else {
        println!("  Recent invoices");
        for i in invoices.iter().take(6) {
            let start = i.period_start.get(0..10).unwrap_or(&i.period_start);
            let end = i.period_end.get(0..10).unwrap_or(&i.period_end);
            println!(
                "    {:<24} {:>9}   {}",
                format!("{start} → {end}"),
                dollars(i.amount_cents),
                i.status
            );
        }
    }
    println!();
    if cost.plan == "hobby" {
        println!("  Upgrade to Pro:  pylon billing upgrade");
    }
    ExitCode::Ok
}

pub fn run_upgrade(args: &[String], json_mode: bool) -> ExitCode {
    let creds = match require_credentials() {
        Ok(c) => c,
        Err(e) => {
            output::print_error(&e);
            eprintln!("  Run: pylon login");
            return ExitCode::Usage;
        }
    };
    let org = match resolve_org(args, &creds) {
        Ok(o) => o,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Usage;
        }
    };

    let return_url = format!("{}/dashboard/orgs/{}/billing", dashboard_url(), org.slug);
    #[derive(Serialize)]
    struct CheckoutArgs<'a> {
        #[serde(rename = "orgId")]
        org_id: &'a str,
        plan: &'a str,
        #[serde(rename = "successUrl")]
        success_url: String,
        #[serde(rename = "cancelUrl")]
        cancel_url: String,
    }
    #[derive(Deserialize)]
    struct CheckoutResp {
        url: String,
    }
    let resp: CheckoutResp = match post_json(
        &creds,
        "/api/fn/createCheckoutSession",
        &CheckoutArgs {
            org_id: &org.id,
            plan: "pro",
            success_url: format!("{return_url}?status=success"),
            cancel_url: return_url.clone(),
        },
    ) {
        Ok(r) => r,
        Err(e) => {
            output::print_error(&format!("Couldn't start checkout: {e}"));
            return ExitCode::Error;
        }
    };

    if json_mode {
        println!("{{\"url\":{:?}}}", resp.url);
        return ExitCode::Ok;
    }
    println!();
    println!("  Opening Stripe checkout for {} → Pro...", org.slug);
    println!("  {}", resp.url);
    let _ = open_browser(&resp.url);
    println!();
    println!("  Complete payment in the browser — your plan updates automatically.");
    ExitCode::Ok
}

/// Best-effort browser open. macOS `open`, Linux `xdg-open`, Windows `start`.
fn open_browser(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn()?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(url).spawn()?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::commands::args::collect_positional;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    fn subcommand(parts: &[&str]) -> Option<String> {
        let args = argv(parts);
        collect_positional(&args, "billing")
            .first()
            .map(|s| s.to_string())
    }

    #[test]
    fn upgrade_routes_to_the_plan_change() {
        assert_eq!(
            subcommand(&["billing", "upgrade"]).as_deref(),
            Some("upgrade")
        );
        assert_eq!(
            subcommand(&["billing", "upgrade", "--org", "acme"]).as_deref(),
            Some("upgrade")
        );
    }

    #[test]
    fn bare_billing_shows_the_summary() {
        assert_eq!(subcommand(&["billing"]), None);
        // The org flag's value must not be read as a subcommand — that
        // would send `pylon billing --org acme` to the checkout.
        assert_eq!(subcommand(&["billing", "--org", "acme"]), None);
        assert_eq!(subcommand(&["billing", "--json"]), None);
    }
}
