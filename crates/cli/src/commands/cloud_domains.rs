//! `pylon domains` — list / add / verify / rm custom domains.

use pylon_kernel::ExitCode;
use serde::Deserialize;

use crate::cloud_client::{post_json, require_credentials, Credentials};
use crate::output;
use crate::project_context::resolve_project_slug;

#[derive(Deserialize)]
struct ProjectIdResponse {
    id: String,
}

#[derive(Deserialize)]
struct Domain {
    id: String,
    hostname: String,
    status: String,
    #[serde(rename = "dnsTarget")]
    dns_target: Option<String>,
    error: Option<String>,
}

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let positional: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with('-') && *a != "domains")
        .map(|s| s.as_str())
        .collect();
    let creds = match require_credentials() {
        Ok(c) => c,
        Err(e) => {
            output::print_error(&e);
            eprintln!("  Run: pylon login");
            return ExitCode::Usage;
        }
    };
    let project_slug = match resolve_project_slug(args, &creds, json_mode) {
        Ok(s) => s,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Usage;
        }
    };
    let project_id = match resolve_project_id(&creds, &project_slug) {
        Ok(id) => id,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Error;
        }
    };
    match positional.first().copied() {
        Some("list") | None => run_list(&creds, &project_id, json_mode),
        Some("add") => run_add(&creds, &project_id, positional.get(1).copied(), json_mode),
        Some("verify") => run_verify(&creds, &project_id, positional.get(1).copied(), json_mode),
        Some("rm") | Some("delete") => run_rm(
            &creds,
            &project_slug,
            positional.get(1).copied(),
            args,
            json_mode,
        ),
        Some(sub) => {
            output::print_error(&format!("unknown subcommand: \"{sub}\""));
            eprintln!("Usage: pylon domains [list | add <host> | verify <host> | rm <host>]");
            ExitCode::Usage
        }
    }
}

fn run_list(creds: &Credentials, project_id: &str, json_mode: bool) -> ExitCode {
    #[derive(serde::Serialize)]
    struct Args<'a> {
        #[serde(rename = "projectId")]
        project_id: &'a str,
    }
    let domains: Vec<Domain> =
        match post_json(creds, "/api/fn/listProjectDomains", &Args { project_id }) {
            Ok(d) => d,
            Err(e) => {
                output::print_error(&e);
                return ExitCode::Error;
            }
        };
    if json_mode {
        let out = serde_json::json!({"domains": domains.iter().map(|d| serde_json::json!({
				"id": d.id, "hostname": d.hostname, "status": d.status,
				"dnsTarget": d.dns_target, "error": d.error,
			})).collect::<Vec<_>>()});
        println!("{}", serde_json::to_string(&out).unwrap_or_default());
        return ExitCode::Ok;
    }
    if domains.is_empty() {
        println!("No custom domains. The default hostname is always available.");
        return ExitCode::Ok;
    }
    let w = domains
        .iter()
        .map(|d| d.hostname.len())
        .max()
        .unwrap_or(8)
        .max(8);
    println!("{:<w$}  STATUS    DNS TARGET", "HOSTNAME", w = w);
    for d in &domains {
        println!(
            "{:<w$}  {:<8}  {}",
            d.hostname,
            d.status,
            d.dns_target.as_deref().unwrap_or("-"),
            w = w
        );
        if let Some(e) = &d.error {
            if !e.is_empty() {
                println!("    ! {e}");
            }
        }
    }
    ExitCode::Ok
}

fn run_add(
    creds: &Credentials,
    project_id: &str,
    host_arg: Option<&str>,
    json_mode: bool,
) -> ExitCode {
    let Some(hostname) = host_arg else {
        output::print_error("Usage: pylon domains add <hostname>");
        return ExitCode::Usage;
    };
    #[derive(serde::Serialize)]
    struct Args<'a> {
        #[serde(rename = "projectId")]
        project_id: &'a str,
        hostname: &'a str,
    }
    #[derive(Deserialize)]
    struct Out {
        #[serde(rename = "dnsTarget")]
        dns_target: Option<String>,
    }
    let r: Out = match post_json(
        creds,
        "/api/fn/addProjectDomain",
        &Args {
            project_id,
            hostname,
        },
    ) {
        Ok(o) => o,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Error;
        }
    };
    if json_mode {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "ok": true,
                "hostname": hostname,
                "dnsTarget": r.dns_target,
            }))
            .unwrap_or_default()
        );
    } else {
        println!("✓ Added {hostname}");
        let recipe =
            find_domain(creds, project_id, hostname).and_then(|d| fetch_recipe(creds, &d.id));
        match recipe {
            Ok(recipe) => print_recipe(&recipe),
            Err(_) => {
                if let Some(t) = &r.dns_target {
                    println!("  Point a CNAME to: {t}");
                }
            }
        }
        println!("  Then: pylon domains verify {hostname}");
    }
    ExitCode::Ok
}

fn run_verify(
    creds: &Credentials,
    project_id: &str,
    host_arg: Option<&str>,
    json_mode: bool,
) -> ExitCode {
    let Some(host_or_id) = host_arg else {
        output::print_error("Usage: pylon domains verify <hostname>");
        return ExitCode::Usage;
    };
    // The control plane keys everything by domain id, while the form `add`
    // suggests is the hostname. Resolve through the listing; a raw id is
    // accepted too.
    let domain = match find_domain(creds, project_id, host_or_id) {
        Ok(d) => d,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Usage;
        }
    };
    #[derive(serde::Serialize)]
    struct Args<'a> {
        #[serde(rename = "domainId")]
        domain_id: &'a str,
    }
    #[derive(Deserialize)]
    struct DnsCheck {
        status: Option<String>,
        #[serde(rename = "pointsAt")]
        points_at: Option<String>,
    }
    #[derive(Deserialize)]
    struct Refresh {
        status: Option<String>,
    }
    let args = Args {
        domain_id: &domain.id,
    };
    // DNS check: does the hostname resolve to the target yet.
    let dns: DnsCheck = match post_json(creds, "/api/fn/verifyDomainDNS", &args) {
        Ok(o) => o,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Error;
        }
    };
    // Certificate check: asks Cloudflare / Fly for the certificate state and
    // moves the domain to `ready` once it is issued. Without it a domain
    // stays `provisioning` in `pylon domains list`.
    // A failure here (a Cloudflare or Fly outage, a role without access)
    // still reports the DNS result.
    let (refreshed, refresh_error) =
        match post_json::<_, Refresh>(creds, "/api/fn/refreshProjectDomain", &args) {
            Ok(r) => (r.status, None),
            Err(e) => (None, Some(e)),
        };
    let domain_status = refreshed.unwrap_or_else(|| domain.status.clone());
    // The records still to add, and the reason it is not ready, when any.
    let (recipe, error) = if domain_status == "ready" {
        (None, None)
    } else {
        let recipe = fetch_recipe(creds, &domain.id).ok();
        let error = find_domain(creds, project_id, &domain.id)
            .ok()
            .and_then(|d| d.error)
            .filter(|e| !e.is_empty());
        (recipe, error)
    };

    if json_mode {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "hostname": domain.hostname,
                "status": dns.status,
                "pointsAt": dns.points_at,
                "domainStatus": domain_status,
                "records": recipe.as_ref().map(DnsRecipe::records).unwrap_or_default(),
                "error": error,
                "refreshError": refresh_error,
            }))
            .unwrap_or_default()
        );
        // JSON callers read the result from the body.
        return ExitCode::Ok;
    }

    let dns_ok = match dns.status.as_deref() {
        Some("ok") => {
            println!("✓ {} — DNS points at the right target", domain.hostname);
            true
        }
        Some("wrong") => {
            println!(
                "✗ {} resolves to {}, expected {}",
                domain.hostname,
                dns.points_at.as_deref().unwrap_or("?"),
                domain.dns_target.as_deref().unwrap_or("the DNS target"),
            );
            false
        }
        Some("missing") => {
            println!(
                "✗ {} — no DNS record found yet (point it at {})",
                domain.hostname,
                domain.dns_target.as_deref().unwrap_or("the DNS target"),
            );
            false
        }
        other => {
            println!("  DNS: {}", other.unwrap_or("?"));
            true
        }
    };
    if let Some(e) = &refresh_error {
        println!("  ! Could not check the certificate: {e}");
    }
    match domain_status.as_str() {
        "ready" => println!("✓ {} — certificate issued, domain ready", domain.hostname),
        status => {
            println!("  Status: {status}");
            if let Some(e) = &error {
                println!("    ! {e}");
            }
            if let Some(recipe) = &recipe {
                print_recipe(recipe);
            }
        }
    }
    if dns_ok && domain_status != "error" {
        ExitCode::Ok
    } else {
        ExitCode::Error
    }
}

/// A domain on the project, by hostname (any case) or id.
fn find_domain(creds: &Credentials, project_id: &str, host_or_id: &str) -> Result<Domain, String> {
    #[derive(serde::Serialize)]
    struct ListArgs<'a> {
        #[serde(rename = "projectId")]
        project_id: &'a str,
    }
    let domains: Vec<Domain> = post_json(
        creds,
        "/api/fn/listProjectDomains",
        &ListArgs { project_id },
    )?;
    domains
        .into_iter()
        .find(|d| d.hostname.eq_ignore_ascii_case(host_or_id) || d.id == host_or_id)
        .ok_or_else(|| {
            format!("no domain \"{host_or_id}\" on this project — run: pylon domains list")
        })
}

/// The DNS records a domain needs, from the control plane's
/// `getProjectDnsRecipe`.
#[derive(Deserialize)]
struct DnsRecipe {
    hostname: String,
    #[serde(rename = "cnameTarget")]
    cname_target: Option<String>,
    #[serde(rename = "txtRecords", default)]
    txt_records: Vec<RecipeRecord>,
    ipv4: Option<String>,
    ipv6: Option<String>,
    #[serde(default)]
    apex: bool,
    #[serde(default)]
    saas: bool,
}

#[derive(Deserialize)]
struct RecipeRecord {
    name: String,
    value: String,
    purpose: String,
    /// `TXT` or `CNAME`, when the control plane sends it.
    #[serde(rename = "type")]
    kind: Option<String>,
}

impl DnsRecipe {
    /// Every record to add, as `{type, name, value, purpose}`.
    fn records(&self) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        let ip_purpose = if self.apex {
            "Route the apex to the app"
        } else {
            "Route the hostname to the app (instead of the CNAME)"
        };
        if !self.apex {
            if let Some(target) = &self.cname_target {
                out.push(serde_json::json!({"type": "CNAME", "name": self.hostname, "value": target, "purpose": "Route the hostname to the app"}));
            }
        }
        // An apex always uses A/AAAA; a hostname off Cloudflare may use them
        // instead of the CNAME.
        if self.apex || !self.saas {
            if let Some(ip) = &self.ipv4 {
                out.push(serde_json::json!({"type": "A", "name": self.hostname, "value": ip, "purpose": ip_purpose}));
            }
            if let Some(ip) = &self.ipv6 {
                out.push(serde_json::json!({"type": "AAAA", "name": self.hostname, "value": ip, "purpose": ip_purpose}));
            }
        }
        for r in &self.txt_records {
            // The control plane lists every proof under txtRecords. Use its
            // `type` when sent; older control planes mark the CNAME (Fly's
            // origin validation) only in the purpose text.
            let kind = match r.kind.as_deref() {
                Some(k) => k,
                None if r.purpose.contains("(CNAME") => "CNAME",
                None => "TXT",
            };
            out.push(serde_json::json!({"type": kind, "name": r.name, "value": r.value, "purpose": r.purpose}));
        }
        out
    }
}

fn fetch_recipe(creds: &Credentials, domain_id: &str) -> Result<DnsRecipe, String> {
    #[derive(serde::Serialize)]
    struct Args<'a> {
        #[serde(rename = "domainId")]
        domain_id: &'a str,
    }
    post_json(creds, "/api/fn/getProjectDnsRecipe", &Args { domain_id })
}

fn print_recipe(recipe: &DnsRecipe) {
    let records = recipe.records();
    if records.is_empty() {
        return;
    }
    println!("  DNS records:");
    for r in &records {
        println!(
            "    {:<5} {}  →  {}",
            r["type"].as_str().unwrap_or(""),
            r["name"].as_str().unwrap_or(""),
            r["value"].as_str().unwrap_or("")
        );
        println!("          ({})", r["purpose"].as_str().unwrap_or(""));
    }
}

fn run_rm(
    creds: &Credentials,
    project_slug: &str,
    host_arg: Option<&str>,
    args: &[String],
    json_mode: bool,
) -> ExitCode {
    let Some(hostname) = host_arg else {
        output::print_error("Usage: pylon domains rm <hostname> [--yes]");
        return ExitCode::Usage;
    };
    if !output::confirm_destructive(args, &format!("remove domain {hostname}"), json_mode) {
        return ExitCode::Usage;
    }
    #[derive(serde::Serialize)]
    struct Args<'a> {
        #[serde(rename = "projectSlug")]
        project_slug: &'a str,
        hostname: &'a str,
    }
    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct Out {
        ok: Option<bool>,
    }
    if let Err(e) = post_json::<_, Out>(
        creds,
        "/api/fn/deleteProjectDomainForCli",
        &Args {
            project_slug,
            hostname,
        },
    ) {
        output::print_error(&e);
        return ExitCode::Error;
    }
    if json_mode {
        println!("{}", serde_json::json!({"ok": true, "hostname": hostname}));
    } else {
        println!("✓ Removed {hostname}");
    }
    ExitCode::Ok
}

fn resolve_project_id(creds: &Credentials, slug: &str) -> Result<String, String> {
    #[derive(serde::Serialize)]
    struct Args<'a> {
        slug: &'a str,
    }
    let proj: ProjectIdResponse = post_json(creds, "/api/fn/getProjectForCli", &Args { slug })
        .map_err(|e| format!("Could not resolve project \"{slug}\": {e}"))?;
    Ok(proj.id)
}

#[cfg(test)]
mod tests {
    use super::DnsRecipe;

    fn recipe(json: serde_json::Value) -> DnsRecipe {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn apex_recipe_lists_a_and_aaaa() {
        let r = recipe(serde_json::json!({
            "hostname": "acme.com", "cnameTarget": "pylon-acme.fly.dev",
            "txtRecords": [], "ipv4": "66.241.124.1", "ipv6": "2a09:8280:1::1", "apex": true
        }));
        let records = r.records();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["type"], "A");
        assert_eq!(records[0]["value"], "66.241.124.1");
        assert_eq!(records[1]["type"], "AAAA");
    }

    #[test]
    fn saas_recipe_types_each_proof_by_its_purpose() {
        let r = recipe(serde_json::json!({
            "hostname": "www.acme.com", "cnameTarget": "customers.stack0.app",
            "txtRecords": [
                {"name": "_acme-challenge.www.acme.com", "value": "tok", "purpose": "Certificate validation"},
                {"name": "_acme-challenge.www.acme.com", "value": "www.acme.com.x.flydns.net.", "purpose": "Origin certificate validation (CNAME, DNS-only)"},
                {"name": "_fly-ownership.www.acme.com", "value": "app-x", "purpose": "Origin certificate ownership (TXT, DNS-only)"}
            ],
            "ipv4": null, "ipv6": null, "saas": true
        }));
        let kinds: Vec<_> = r
            .records()
            .iter()
            .map(|r| r["type"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(kinds, ["CNAME", "TXT", "CNAME", "TXT"]);
    }

    #[test]
    fn an_explicit_record_type_wins_over_the_purpose_text() {
        let r = recipe(serde_json::json!({
            "hostname": "www.acme.com", "cnameTarget": "customers.stack0.app",
            "txtRecords": [
                {"name": "_acme-challenge.www.acme.com", "value": "x.flydns.net.", "purpose": "Origin validation", "type": "CNAME"}
            ],
            "saas": true
        }));
        assert_eq!(r.records()[1]["type"], "CNAME");
    }

    #[test]
    fn a_hostname_off_cloudflare_lists_the_ips_as_an_alternative() {
        let r = recipe(serde_json::json!({
            "hostname": "app.acme.com", "cnameTarget": "pylon-acme.fly.dev",
            "txtRecords": [], "ipv4": "66.241.124.1", "ipv6": null, "saas": false
        }));
        let kinds: Vec<_> = r
            .records()
            .iter()
            .map(|r| r["type"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(kinds, ["CNAME", "A"]);
    }
}
