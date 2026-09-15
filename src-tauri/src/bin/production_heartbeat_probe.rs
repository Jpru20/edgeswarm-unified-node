use edgeswarm_unified_node_lib::core::{
    capacity::CapacityStatus,
    production_heartbeat::ProductionHeartbeatV1,
    NodeState,
};

fn main() {
    let state = NodeState::detect();

    let heartbeat =
        ProductionHeartbeatV1::from_node_state(
            &state,
            env!("CARGO_PKG_VERSION"),
            "laptop",
            &[],
        );

    println!(
        "{}",
        serde_json::to_string_pretty(&heartbeat)
            .expect("heartbeat serialization failed")
    );

    println!();
    println!(
        "MODELS_AVAILABLE_COUNT={}",
        heartbeat.models_available.len()
    );

    println!(
        "ELIGIBLE_CAPABILITY_COUNT={}",
        heartbeat
            .eligible_model_capabilities
            .len()
    );

    println!(
        "PRIMARY_MODEL={}",
        heartbeat
            .model_id
            .as_deref()
            .unwrap_or("none")
    );

    println!(
        "PRODUCTION_CONCURRENCY_LIMIT={}",
        heartbeat.concurrency_limit
    );

    println!(
        "DISK_FREE_GB={}",
        heartbeat.disk_free_gb
    );

    println!(
        "UPTIME_SEC={}",
        heartbeat.uptime_sec
    );

    println!(
        "GPU_VENDOR={}",
        heartbeat
            .gpu_vendor
            .as_deref()
            .unwrap_or("none")
    );

    println!(
        "GPU_MEMORY_MB={}",
        heartbeat
            .gpu_memory_mb
            .unwrap_or(0)
    );

    println!(
        "CUDA_AVAILABLE={}",
        heartbeat.cuda_available
    );

    println!(
        "VULKAN_AVAILABLE={}",
        heartbeat.vulkan_available
    );

    println!(
        "METAL_AVAILABLE={}",
        heartbeat.metal_available
    );

    println!(
        "MODEL_SIZE_GB={}",
        heartbeat
            .model_size_gb
            .unwrap_or(0.0)
    );

    assert!(
        !heartbeat.hardware_id.trim().is_empty()
    );

    assert!(
        !heartbeat.cpu_name.trim().is_empty()
    );

    assert!(
        heartbeat.ram_gb > 0.0
    );

    assert!(
        heartbeat.disk_free_gb > 0
    );

    assert!(
        heartbeat.concurrency_limit >= 1
            && heartbeat.concurrency_limit <= 5
    );

    if let Some(primary) =
        heartbeat.model_id.as_deref()
    {
        let capacity = heartbeat
            .metadata
            .model_capacity_v1
            .iter()
            .find(|model| {
                model.selected_model == primary
            })
            .expect(
                "primary model missing from capacity metadata"
            );

        assert_eq!(
            capacity.capacity_status,
            CapacityStatus::Certified
        );

        let certified =
            capacity.certified_concurrency
                .expect(
                    "primary certified concurrency missing"
                )
                .max(1)
                .min(5);

        assert_eq!(
            heartbeat.concurrency_limit,
            certified
        );

        assert!(
            heartbeat
                .model_size_gb
                .unwrap_or(0.0) > 0.0
        );

        assert!(
            heartbeat.models_available
                .iter()
                .any(|model| model == primary)
        );
    }

    println!(
        "MODEL_CAPACITY_ENTRY_COUNT={}",
        heartbeat
            .metadata
            .model_capacity_v1
            .len()
    );

    println!(
        "PRODUCTION_HEARTBEAT_PAYLOAD_VALID=true"
    );
}
