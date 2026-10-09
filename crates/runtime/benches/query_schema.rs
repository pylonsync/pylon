use std::hint::black_box;
use std::time::Instant;

use pylon_kernel::{AppManifest, ManifestEntity, ManifestField};
use pylon_runtime::Runtime;

fn measure(name: &str, mut read: impl FnMut()) {
    for _ in 0..100 {
        read();
    }
    let mut samples = Vec::new();
    for _ in 0..9 {
        let start = Instant::now();
        for _ in 0..2000 {
            read();
        }
        samples.push(start.elapsed().as_secs_f64() * 1e6 / 2000.0);
    }
    samples.sort_by(f64::total_cmp);
    println!("{name}: {:.3} us/read median", samples[4]);
}

fn main() {
    for width in [8, 200] {
        let fields = (0..width)
            .map(|i| ManifestField {
                name: format!("field_{i}"),
                field_type: "string".into(),
                optional: true,
                ..Default::default()
            })
            .collect();
        let runtime = Runtime::in_memory(AppManifest {
            name: "query-schema-bench".into(),
            entities: vec![ManifestEntity {
                name: "Record".into(),
                fields,
                ..Default::default()
            }],
            ..Default::default()
        })
        .unwrap();
        let id = runtime
            .insert("Record", &serde_json::json!({ "field_0": "value" }))
            .unwrap();
        measure(&format!("{width} fields, missing ID"), || {
            assert!(black_box(runtime.get_by_id("Record", "missing").unwrap()).is_none());
        });
        measure(&format!("{width} fields, existing ID"), || {
            assert!(black_box(runtime.get_by_id("Record", &id).unwrap()).is_some());
        });
        let filter = serde_json::json!({ "id": { "$in": (0..1000).map(|i| format!("missing-{i}")).collect::<Vec<_>>() } });
        measure(&format!("{width} fields, 1000-ID filter"), || {
            assert!(black_box(runtime.query_filtered("Record", &filter).unwrap()).is_empty());
        });
    }
}
