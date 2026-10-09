use std::{hint::black_box, time::Instant};

use pylon_runtime::workflows::{WorkflowDef, WorkflowEngine, WorkflowFilter, WorkflowStatus};
use serde_json::json;

fn main() {
    let engine = WorkflowEngine::new("http://127.0.0.1:1", 5000);
    engine.register(WorkflowDef {
        name: "bench".into(),
        description: String::new(),
        file: String::new(),
        max_retries: 0,
        step_timeout_secs: 30,
    });
    for _ in 0..5000 {
        let id = engine.start("bench", json!({})).unwrap();
        engine
            .advance_with_response(
                &id,
                json!({
                    "action":"step_complete", "step_name":"work", "output":"x".repeat(16384),
                }),
            )
            .unwrap();
    }
    let filter = WorkflowFilter {
        limit: 100,
        ..Default::default()
    };
    for include_steps in [false, true] {
        let mut samples = Vec::new();
        for _ in 0..7 {
            let start = Instant::now();
            for _ in 0..5 {
                let rows = black_box(engine.list_filtered(&filter, include_steps).unwrap());
                assert_eq!(rows.len(), 100);
                assert_eq!(rows[0].steps.len(), usize::from(include_steps));
            }
            samples.push(start.elapsed().as_secs_f64() * 1e3 / 5.0);
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "5000 runs, 16 KiB step output, include_steps={include_steps}: {:.3} ms/list median",
            samples[3]
        );
    }
    for count_only in [false, true] {
        let mut samples = Vec::new();
        for _ in 0..7 {
            let start = Instant::now();
            for _ in 0..5 {
                let running = if count_only {
                    engine.status_counts().running
                } else {
                    engine
                        .list(None)
                        .iter()
                        .filter(|workflow| workflow.status == WorkflowStatus::Running)
                        .count() as u64
                };
                assert_eq!(black_box(running), 5000);
            }
            samples.push(start.elapsed().as_secs_f64() * 1e3 / 5.0);
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "5000 runs, workflow metrics, count_only={count_only}: {:.3} ms median",
            samples[3]
        );
    }
}
