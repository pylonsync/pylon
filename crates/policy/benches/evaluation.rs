use std::{hint::black_box, time::Instant};

use pylon_auth::AuthContext;
use pylon_kernel::{AppManifest, ManifestPolicy};
use pylon_policy::PolicyEngine;
use serde_json::json;

fn main() {
    let auth = AuthContext::user("user".into()).with_tenant("tenant".into());
    let row = json!({"orgId":"tenant", "expiresAt":"2999-01-01T00:00:00Z"});
    for rule in [
        "true",
        "auth.tenantId == data.orgId",
        "data.expiresAt > now",
    ] {
        let engine = PolicyEngine::from_manifest(&AppManifest {
            policies: vec![ManifestPolicy {
                name: "read".into(),
                entity: Some("Record".into()),
                allow_read: Some(rule.into()),
                ..Default::default()
            }],
            ..Default::default()
        });
        let mut samples = Vec::new();
        for _ in 0..7 {
            let start = Instant::now();
            for _ in 0..100_000 {
                assert!(
                    black_box(engine.check_entity_read("Record", &auth, Some(&row))).is_allowed()
                );
            }
            samples.push(start.elapsed().as_secs_f64() * 1e9 / 100_000.0);
        }
        samples.sort_by(f64::total_cmp);
        println!("{rule}: {:.1} ns/check median", samples[3]);
    }
}
