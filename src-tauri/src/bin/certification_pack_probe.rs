use edgeswarm_unified_node_lib::core::certification_workload::built_in_3b_realworld_v2;

fn main() {
    let pack = built_in_3b_realworld_v2()
        .expect("3B v2 certification pack must be valid");

    println!("PACK_ID={}", pack.pack_id);
    println!("PACK_VERSION={}", pack.pack_version);
    println!("MODEL_TIER={}", pack.model_tier);
    println!("WORKLOADS={}", pack.workloads.len());

    for workload in &pack.workloads {
        println!(
            "{} | route={} | lane={}",
            workload.id,
            workload.expected_required_model,
            workload.adapter_lane
        );
    }
}
