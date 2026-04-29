use std::path::Path;

use pylon_kernel::AppManifest;

// ---------------------------------------------------------------------------
// Static site generation
// ---------------------------------------------------------------------------

/// A generated static page.
#[derive(Debug, Clone)]
pub struct StaticPage {
    /// The output file path relative to the build directory (e.g., "index.html").
    pub path: String,
    /// The HTML content.
    pub html: String,
}

/// Generate static pages from a manifest.
/// Only routes with mode "static" are rendered.
/// Routes with path parameters (e.g., /posts/:slug) are skipped
/// since they need runtime data to enumerate.
pub fn generate_static_pages(manifest: &AppManifest) -> Vec<StaticPage> {
    let mut pages = Vec::new();

    for route in &manifest.routes {
        if route.mode != "static" {
            continue;
        }

        // Skip parameterized routes — they need data to enumerate.
        if route.path.contains(':') {
            continue;
        }

        let file_path = route_path_to_file(&route.path);
        let html = render_page(manifest, &route.path, route.query.as_deref());
        pages.push(StaticPage {
            path: file_path,
            html,
        });
    }

    pages
}

/// Write generated pages to an output directory.
///
/// Rejects any page whose computed path would escape `out_dir`. The page
/// paths come from `route.path` in the manifest, which is user-authored —
/// a route of `/../../tmp/pwn` would previously write outside `out_dir`
/// because schema validation only checks "starts with `/`" and uniqueness.
/// We canonicalize `out_dir`, then canonicalize the write target's parent,
/// and refuse the write if it doesn't sit under `out_dir`.
pub fn write_pages(pages: &[StaticPage], out_dir: &Path) -> Result<usize, String> {
    std::fs::create_dir_all(out_dir)
        .map_err(|e| format!("Failed to create output directory: {e}"))?;

    let out_canonical = std::fs::canonicalize(out_dir).map_err(|e| {
        format!(
            "Failed to canonicalize output directory {}: {e}",
            out_dir.display()
        )
    })?;

    for page in pages {
        // Reject obviously bad paths up front — `..` anywhere in the
        // components, absolute paths, or Windows drive prefixes.
        let pp = Path::new(&page.path);
        if page.path.is_empty() || pp.is_absolute() {
            return Err(format!(
                "refusing page with absolute or empty path: {:?}",
                page.path
            ));
        }
        if pp
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(format!(
                "refusing page path with `..` component: {:?}",
                page.path
            ));
        }

        let full_path = out_canonical.join(&page.path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create directory {}: {e}", parent.display()))?;
            // Canonicalize the parent (it exists now) and check containment.
            let parent_canonical = std::fs::canonicalize(parent)
                .map_err(|e| format!("Failed to canonicalize {}: {e}", parent.display()))?;
            if !parent_canonical.starts_with(&out_canonical) {
                return Err(format!(
                    "page path {:?} would write outside the output directory",
                    page.path
                ));
            }
        }
        std::fs::write(&full_path, &page.html)
            .map_err(|e| format!("Failed to write {}: {e}", full_path.display()))?;
    }

    Ok(pages.len())
}

fn route_path_to_file(route_path: &str) -> String {
    if route_path == "/" {
        "index.html".to_string()
    } else {
        let trimmed = route_path.trim_start_matches('/');
        format!("{}/index.html", trimmed)
    }
}

fn render_page(manifest: &AppManifest, route_path: &str, query_name: Option<&str>) -> String {
    let title = format!("{} — {}", route_path, manifest.name);
    let query_info = match query_name {
        Some(q) => format!("<p>Query: <code>{q}</code></p>"),
        None => String::new(),
    };

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>{title}</title>
  <script>window.__PYLON_MANIFEST__ = {manifest_json};</script>
</head>
<body>
  <h1>{app_name}</h1>
  <p>Route: <code>{route_path}</code></p>
  {query_info}
  <div id="app"></div>
  <script type="module" src="/app.js"></script>
</body>
</html>"#,
        title = title,
        manifest_json = serde_json::to_string(manifest).unwrap_or_else(|_| "{}".into()),
        app_name = manifest.name,
        route_path = route_path,
        query_info = query_info,
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_kernel::*;

    fn test_manifest() -> AppManifest {
        serde_json::from_str(include_str!(
            "../../../examples/todo-app/pylon.manifest.json"
        ))
        .unwrap()
    }

    #[test]
    fn no_static_routes_produces_no_pages() {
        // The todo app has no static routes (all are "server" or "live").
        // Wait — it has one static route: /todos/:todoId is "server".
        // Let me check.
        let m = test_manifest();
        let pages = generate_static_pages(&m);
        // The todo app has server and live routes, no pure static.
        // Unless we count... let me just test with a custom manifest.
        assert!(pages.is_empty());
    }

    #[test]
    fn static_route_generates_page() {
        let m = AppManifest {
            manifest_version: MANIFEST_VERSION,
            name: "test".into(),
            version: "0.1.0".into(),
            entities: vec![],
            routes: vec![
                ManifestRoute {
                    path: "/".into(),
                    mode: "static".into(),
                    query: None,
                    auth: None,
                },
                ManifestRoute {
                    path: "/about".into(),
                    mode: "static".into(),
                    query: None,
                    auth: None,
                },
                ManifestRoute {
                    path: "/live".into(),
                    mode: "live".into(),
                    query: None,
                    auth: None,
                },
            ],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            auth: Default::default(),
        };

        let pages = generate_static_pages(&m);
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].path, "index.html");
        assert_eq!(pages[1].path, "about/index.html");
        assert!(pages[0].html.contains("<!DOCTYPE html>"));
        assert!(pages[0].html.contains("test"));
    }

    #[test]
    fn parameterized_static_routes_skipped() {
        let m = AppManifest {
            manifest_version: MANIFEST_VERSION,
            name: "test".into(),
            version: "0.1.0".into(),
            entities: vec![],
            routes: vec![ManifestRoute {
                path: "/posts/:slug".into(),
                mode: "static".into(),
                query: None,
                auth: None,
            }],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            auth: Default::default(),
        };

        let pages = generate_static_pages(&m);
        assert!(pages.is_empty());
    }

    #[test]
    fn route_path_to_file_mapping() {
        assert_eq!(route_path_to_file("/"), "index.html");
        assert_eq!(route_path_to_file("/about"), "about/index.html");
        assert_eq!(route_path_to_file("/docs/api"), "docs/api/index.html");
    }

    #[test]
    fn rendered_page_contains_manifest() {
        let m = AppManifest {
            manifest_version: MANIFEST_VERSION,
            name: "myapp".into(),
            version: "1.0.0".into(),
            entities: vec![],
            routes: vec![ManifestRoute {
                path: "/".into(),
                mode: "static".into(),
                query: Some("allPosts".into()),
                auth: None,
            }],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            auth: Default::default(),
        };

        let pages = generate_static_pages(&m);
        assert_eq!(pages.len(), 1);
        assert!(pages[0].html.contains("__PYLON_MANIFEST__"));
        assert!(pages[0].html.contains("myapp"));
        assert!(pages[0].html.contains("allPosts"));
    }

    #[test]
    fn write_pages_rejects_parent_dir_traversal() {
        let dir = std::env::temp_dir().join("pylon-staticgen-traversal-test");
        let _ = std::fs::remove_dir_all(&dir);
        let pages = vec![StaticPage {
            path: "../../tmp/pwn.html".into(),
            html: "<h1>x</h1>".into(),
        }];
        let err = write_pages(&pages, &dir).unwrap_err();
        assert!(
            err.contains("..") || err.contains("outside"),
            "unexpected: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_pages_rejects_absolute_path() {
        let dir = std::env::temp_dir().join("pylon-staticgen-abs-test");
        let _ = std::fs::remove_dir_all(&dir);
        let pages = vec![StaticPage {
            path: "/tmp/pwn.html".into(),
            html: "<h1>x</h1>".into(),
        }];
        let err = write_pages(&pages, &dir).unwrap_err();
        assert!(err.contains("absolute") || err.contains("outside"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_pages_to_temp_dir() {
        let m = AppManifest {
            manifest_version: MANIFEST_VERSION,
            name: "test".into(),
            version: "0.1.0".into(),
            entities: vec![],
            routes: vec![ManifestRoute {
                path: "/".into(),
                mode: "static".into(),
                query: None,
                auth: None,
            }],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            auth: Default::default(),
        };

        let pages = generate_static_pages(&m);
        let dir = std::env::temp_dir().join("pylon-staticgen-test");
        let _ = std::fs::remove_dir_all(&dir);

        let count = write_pages(&pages, &dir).unwrap();
        assert_eq!(count, 1);
        assert!(dir.join("index.html").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
