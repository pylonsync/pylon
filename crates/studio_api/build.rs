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
    // Watch the source so a `cargo build` after a JS change picks up
    // the stale-bundle warning (the `include_str!` will use the old
    // dist, but Cargo will at least re-run this script and reprint
    // the rerun-if-changed list — useful in IDEs).
    println!("cargo:rerun-if-changed=web/src");
    println!("cargo:rerun-if-changed=web/index.html");
    println!("cargo:rerun-if-changed=web/vite.config.ts");
}
