use std::fs;
use std::time::Instant;
use tempfile::tempdir;
use threadmoth::engine::compute_sha256;
use threadmoth::pipeline::{apply_prepared_plan, prepare_request_plan};
use threadmoth::protocol::*;
use threadmoth::provider::text::TextOperation;
use threadmoth::workspace::Workspace;

#[test]
fn profile_1mb_edit_phases() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let workspace = Workspace::new(root).unwrap();

    let target_rel = "large_1mb.txt";
    let target_path = root.join(target_rel);

    // Create 1 MB file with unique target in the middle
    let mut content = Vec::with_capacity(1_000_000);
    let half = 500_000;
    content.extend(vec![b'a'; half]);
    content.extend(b"\nTARGET_LINE_TO_REPLACE=OLD_VALUE\n");
    content.extend(vec![b'b'; half]);
    fs::write(&target_path, &content).unwrap();

    println!("\n=== OPTIMIZED PROFILING 1 MB EDIT PHASES ===");
    println!("File size: {} bytes", content.len());

    let iterations = 10;
    let mut read_us = Vec::new();
    let mut hash_us = Vec::new();
    let mut prep_plan_us = Vec::new();
    let mut apply_plan_us = Vec::new();
    let mut total_us = Vec::new();

    for i in 0..iterations {
        // Reset file content
        fs::write(&target_path, &content).unwrap();

        let t_total_start = Instant::now();

        // 1. File read
        let t_read_start = Instant::now();
        let bytes = workspace.read_file(&target_rel.to_string()).unwrap();
        let t_read = t_read_start.elapsed().as_micros();

        // 2. Hash
        let t_hash_start = Instant::now();
        let file_hash = compute_sha256(&bytes);
        let t_hash = t_hash_start.elapsed().as_micros();

        // 3. Prepare Plan
        let request = Request {
            version: "1.4.0".into(),
            request_id: format!("req-{}", i),
            allow_generated: false,
            allow_newline_conversion: false,
            file_path: target_rel.to_string(),
            namespace: threadmoth::path::PathNamespace::Native,
            expected_pre_hash: Some(file_hash),
            region_guard: None,
            candidate_guard: None,
            cardinality: Cardinality::ExactlyOne,
            budget: EffectBudget::default(),
            operation: OperationPayload::Text(TextOperation::Replace {
                target: "TARGET_LINE_TO_REPLACE=OLD_VALUE".into(),
                replacement: "TARGET_LINE_TO_REPLACE=NEW_VALUE".into(),
            }),
        };

        let t_prep_start = Instant::now();
        let plan = prepare_request_plan(&workspace, &request, Vec::new()).unwrap();
        let t_prep = t_prep_start.elapsed().as_micros();

        // 4. Apply Plan
        let t_apply_start = Instant::now();
        let cert_res = apply_prepared_plan(&workspace, &plan);
        let t_apply = t_apply_start.elapsed().as_micros();

        let t_total = t_total_start.elapsed().as_micros();

        read_us.push(t_read);
        hash_us.push(t_hash);
        prep_plan_us.push(t_prep);
        apply_plan_us.push(t_apply);
        total_us.push(t_total);

        // Verify result
        match cert_res {
            PlanApplyResult::Certificate(c) => {
                assert_eq!(c.outcome, Outcome::Applied);
            }
            PlanApplyResult::Transaction(_) => panic!("Expected single certificate"),
        }
    }

    let avg = |v: &[u128]| v.iter().sum::<u128>() as f64 / v.len() as f64;
    println!("Phase 1: workspace.read_file() : {:.2} us ({:.3} ms)", avg(&read_us), avg(&read_us)/1000.0);
    println!("Phase 2: compute_sha256()      : {:.2} us ({:.3} ms)", avg(&hash_us), avg(&hash_us)/1000.0);
    println!("Phase 3: prepare_request_plan(): {:.2} us ({:.3} ms)", avg(&prep_plan_us), avg(&prep_plan_us)/1000.0);
    println!("Phase 4: apply_prepared_plan() : {:.2} us ({:.3} ms)", avg(&apply_plan_us), avg(&apply_plan_us)/1000.0);
    println!("TOTAL In-Process Edit Duration : {:.2} us ({:.3} ms)", avg(&total_us), avg(&total_us)/1000.0);
}
