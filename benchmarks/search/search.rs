//! Search throughput + latency bench.
//!
//! Drives `Runtime::search` directly (no HTTP, no WebSocket, no
//! function-runner round-trip). Tells you the storage-layer ceiling.
//!
//! Run: cargo bench --manifest-path benchmarks/search/Cargo.toml
use std::time::Instant;

use pylon_http::DataStore;
use pylon_kernel::{AppManifest, ManifestEntity, ManifestField, ManifestSearchConfig};
use pylon_runtime::Runtime;
use rand::seq::SliceRandom;
use rand::Rng;
use serde_json::json;

const BRANDS: &[&str] = &["Atlas", "Orbit", "Nimbus", "Forge", "Quill", "Relay"];
const CATEGORIES: &[&str] = &["Shoes", "Shirts", "Jackets", "Watches", "Bags"];
const COLORS: &[&str] = &["red", "blue", "green", "black", "white"];
const ADJ: &[&str] = &["lightweight", "rugged", "minimalist", "vintage", "premium"];
const NOUN: &[&str] = &["cruiser", "runner", "trainer", "shirt", "tote"];

fn f(name: &str, ty: &str) -> ManifestField {
    ManifestField {
        name: name.into(),
        field_type: ty.into(),
        optional: false,
        unique: false,
    }
}

fn build_manifest() -> AppManifest {
    let entity = ManifestEntity {
        name: "Product".into(),
        fields: vec![
            f("name", "string"),
            f("description", "richtext"),
            f("brand", "string"),
            f("category", "string"),
            f("color", "string"),
            f("price", "float"),
            f("rating", "float"),
            f("stock", "int"),
            f("createdAt", "datetime"),
        ],
        indexes: vec![],
        relations: vec![],
        search: Some(ManifestSearchConfig {
            text: vec!["name".into(), "description".into()],
            facets: vec!["brand".into(), "category".into(), "color".into()],
            sortable: vec!["price".into(), "rating".into(), "createdAt".into()],
        }),
    };
    AppManifest {
        manifest_version: 1,
        name: "search-bench".into(),
        version: "0.0.0".into(),
        entities: vec![entity],
        queries: vec![],
        actions: vec![],
        policies: vec![],
        routes: vec![],
    }
}

fn seed(rt: &Runtime, count: usize) {
    let mut rng = rand::thread_rng();
    for _i in 0..count {
        let brand = BRANDS.choose(&mut rng).unwrap();
        let category = CATEGORIES.choose(&mut rng).unwrap();
        let color = COLORS.choose(&mut rng).unwrap();
        let adj = ADJ.choose(&mut rng).unwrap();
        let noun = NOUN.choose(&mut rng).unwrap();
        let name = format!("{} {} {}", brand, adj, noun);
        let description = format!(
            "A {color} {category} for everyday wear. {adj} construction with a soft feel."
        );
        rt.insert(
            "Product",
            &json!({
                "name": name,
                "description": description,
                "brand": brand,
                "category": category,
                "color": color,
                "price": rng.gen_range(1000..50000) as f64 / 100.0,
                "rating": rng.gen_range(30..50) as f64 / 10.0,
                "stock": rng.gen_range(0..50),
                "createdAt": "2026-01-01T00:00:00Z",
            }),
        )
        .unwrap_or_else(|e| panic!("seed insert failed: {e}"));
    }
}

fn bench<F: FnMut()>(name: &str, iterations: u32, mut f: F) {
    for _ in 0..32 {
        f();
    }
    let start = Instant::now();
    for _ in 0..iterations {
        f();
    }
    let elapsed = start.elapsed();
    let per_op = elapsed / iterations;
    let ops_sec = if per_op.as_nanos() > 0 {
        1_000_000_000 / per_op.as_nanos()
    } else {
        0
    };
    println!(
        "  {:<48} {:>8} iters {:>10.2?} total {:>9.2?}/op {:>8} ops/sec",
        name, iterations, elapsed, per_op, ops_sec
    );
}

fn main() {
    let manifest = build_manifest();
    println!("[search-bench] seeding 10K rows…");
    let rt_10k = Runtime::in_memory(manifest.clone()).unwrap();
    rt_10k.ensure_search_indexes().unwrap();
    seed(&rt_10k, 10_000);

    println!("[search-bench] seeding 100K rows…");
    let rt_100k = Runtime::in_memory(manifest.clone()).unwrap();
    rt_100k.ensure_search_indexes().unwrap();
    seed(&rt_100k, 100_000);

    println!("\n=== 10K rows ===");
    bench("empty query, page 0", 10_000, || {
        rt_10k
            .search(
                "Product",
                &json!({
                    "query": "",
                    "facets": ["brand", "category", "color"],
                    "page": 0,
                    "pageSize": 24,
                }),
            )
            .unwrap();
    });
    bench("text 'red'", 10_000, || {
        rt_10k
            .search(
                "Product",
                &json!({
                    "query": "red",
                    "facets": ["brand", "category", "color"],
                    "page": 0,
                    "pageSize": 24,
                }),
            )
            .unwrap();
    });
    bench("filter brand+category", 10_000, || {
        rt_10k
            .search(
                "Product",
                &json!({
                    "query": "",
                    "filters": {"brand": "Atlas", "category": "Shoes"},
                    "facets": ["color"],
                    "page": 0,
                    "pageSize": 24,
                }),
            )
            .unwrap();
    });
    bench("sort price asc, page 5", 10_000, || {
        rt_10k
            .search(
                "Product",
                &json!({
                    "query": "",
                    "sort": ["price", "asc"],
                    "page": 5,
                    "pageSize": 24,
                }),
            )
            .unwrap();
    });

    println!("\n=== 100K rows ===");
    bench("empty query, page 0", 5_000, || {
        rt_100k
            .search(
                "Product",
                &json!({
                    "query": "",
                    "facets": ["brand", "category", "color"],
                    "page": 0,
                    "pageSize": 24,
                }),
            )
            .unwrap();
    });
    bench("text 'red'", 5_000, || {
        rt_100k
            .search(
                "Product",
                &json!({
                    "query": "red",
                    "facets": ["brand", "category", "color"],
                    "page": 0,
                    "pageSize": 24,
                }),
            )
            .unwrap();
    });
    bench("filter brand", 5_000, || {
        rt_100k
            .search(
                "Product",
                &json!({
                    "query": "",
                    "filters": {"brand": "Atlas"},
                    "page": 0,
                    "pageSize": 24,
                }),
            )
            .unwrap();
    });
}
