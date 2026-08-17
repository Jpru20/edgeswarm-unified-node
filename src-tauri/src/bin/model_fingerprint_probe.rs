use edgeswarm_unified_node_lib::core::
    model_fingerprint::resolve_model_fingerprint;
use std::path::Path;

fn main() {
    let selected_model = std::env::args()
        .nth(1)
        .expect("selected model required");

    let model_path = std::env::args()
        .nth(2)
        .expect("model path required");

    let forced = resolve_model_fingerprint(
        &selected_model,
        Path::new(&model_path),
        true,
    ).expect("forced fingerprint failed");

    let cached = resolve_model_fingerprint(
        &selected_model,
        Path::new(&model_path),
        false,
    ).expect("cached fingerprint failed");

    println!("SELECTED_MODEL={}", forced.selected_model);
    println!("MODEL_PATH={}", forced.canonical_path);
    println!("FILE_SIZE={}", forced.file_size);
    println!("SHA256={}", forced.sha256);
    println!("FORCED_CACHE_HIT={}", forced.cache_hit);
    println!("SECOND_CACHE_HIT={}", cached.cache_hit);

    assert!(!forced.cache_hit);
    assert!(cached.cache_hit);
    assert_eq!(forced.sha256, cached.sha256);

    println!("FINGERPRINT_CACHE_REUSE_VALID=true");
}
