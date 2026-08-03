//! Cargo build script for pylon-studio-api.
//!
//! Studio's UI lives in `web/` (Vite + React + shadcn) and produces a
//! single self-contained HTML at `web/dist/index.html`, which the
//! crate embeds via `include_str!`. We don't run `bun run build` here
//! because that adds a Bun dependency to every `cargo build` (CI, IDE
//! save-on-build, etc.) and silently masks Studio drift if the
//! sub-build fails.
//!
//! Instead this script:
//!   1. Re-runs Cargo when any `web/src/` file or the bundle changes
//!      so a stale bundle gets noticed.
//!   2. Hard-fails the crate build if `web/dist/index.html` is
//!      missing — the operator gets a clear "go run `bun run build`"
//!      message instead of a confusing `include_str!` error.

use std::path::Path;

fn main() {
    let dist = Path::new("web/dist/index.html");
    if !dist.exists() {
        // Fail loud, with the fix instructions inline. No need to scour
        // the docs for what to run.
        panic!(
            "\n\npylon-studio-api: web/dist/index.html is missing.\n\
             The Studio UI is a Vite + React build that needs to be \n\
             produced before this crate can be compiled. Run:\n\n\
             \t(cd crates/studio_api/web && bun install && bun run build)\n\n\
             then re-run cargo build.\n",
        );
    }
    println!("cargo:rerun-if-changed=web/dist/index.html");
    // Watch the source so a `cargo build` after a JS change re-runs this
    // script and re-checks staleness below.
    println!("cargo:rerun-if-changed=web/src");
    println!("cargo:rerun-if-changed=web/index.html");
    println!("cargo:rerun-if-changed=web/vite.config.ts");

    warn_if_stale(dist);
}

/// Warn when `web/src` is newer than the bundle we're about to embed.
///
/// `include_str!` happily embeds whatever is on disk, and nothing else notices:
/// CI, the Dockerfile and release.yml all run `bun run build` first, so the
/// published binary is always current, while a local `cargo build` silently
/// ships whatever bundle was last built there. That gap has been months wide.
///
/// It stopped being cosmetic when Studio moved to session auth: a stale bundle
/// still renders the old "paste your PYLON_ADMIN_TOKEN" dialog and posts it to
/// a `/studio/login` route the server no longer serves.
///
/// A warning rather than a hard failure, because mtimes aren't reliable
/// everywhere this builds — a fresh `git clone` and a `cargo publish` verify
/// (which unpacks the tarball, dist included) can both produce source newer
/// than dist through no fault of the operator. A missing bundle still panics;
/// that check is unambiguous.
fn warn_if_stale(dist: &Path) {
    let newest_src = ["web/src", "web/index.html", "web/vite.config.ts"]
        .iter()
        .filter_map(|p| newest_mtime(Path::new(p)))
        .max();
    let (Some(src), Some(built)) = (newest_src, mtime(dist)) else {
        return;
    };
    if src > built {
        println!(
            "cargo:warning=pylon-studio-api: web/dist/index.html is OLDER than web/src — \
             the embedded Studio bundle does not include your latest UI changes. \
             Run `(cd crates/studio_api/web && bun run build)` and rebuild."
        );
    }
}

fn mtime(p: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(p).ok()?.modified().ok()
}

/// Newest mtime in a file or directory tree. `None` if the path is missing.
fn newest_mtime(p: &Path) -> Option<std::time::SystemTime> {
    if p.is_file() {
        return mtime(p);
    }
    let mut newest = mtime(p);
    for entry in std::fs::read_dir(p).ok()?.flatten() {
        if let Some(t) = newest_mtime(&entry.path()) {
            newest = Some(newest.map_or(t, |cur| cur.max(t)));
        }
    }
    newest
}
