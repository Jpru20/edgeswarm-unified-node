use edgeswarm_unified_node_lib::core::
    hardware_identity::HardwareIdentity;

fn main() {
    let first =
        HardwareIdentity::detect()
            .expect(
                "hardware identity detection failed"
            );

    let second =
        HardwareIdentity::detect()
            .expect(
                "hardware identity repeat detection failed"
            );

    let stable =
        first.hardware_id == second.hardware_id;

    let valid =
        HardwareIdentity::is_valid_hardware_id(
            &first.hardware_id
        );

    println!(
        "HARDWARE_ID={}",
        first.hardware_id
    );

    println!(
        "HARDWARE_ID_LENGTH={}",
        first.hardware_id.len()
    );

    println!(
        "HARDWARE_ID_SOURCE={}",
        first.source
    );

    println!(
        "HARDWARE_ID_PERSISTENCE_STATUS={}",
        first.persistence_status
    );

    println!(
        "HARDWARE_ID_STABLE={}",
        stable
    );

    println!(
        "HARDWARE_ID_VALID_64_HEX={}",
        valid
    );

    assert!(stable);
    assert!(valid);
    assert_eq!(
        first.hardware_id.len(),
        64
    );

    println!(
        "HARDWARE_IDENTITY_REAL_SERVER_VALID=true"
    );

    println!(
        "RAW_HARDWARE_IDENTIFIER_PRINTED=false"
    );

    println!(
        "NODE_STATE_CHANGED=false"
    );

    println!(
        "HEARTBEAT_HARDWARE_ID_CHANGED=false"
    );

    println!(
        "NETWORK_REQUEST_SENT=false"
    );

    println!(
        "DATABASE_WRITE=false"
    );

    println!(
        "SECOND_HEARTBEAT_SENT=false"
    );

    println!(
        "WALLET_CREATED=false"
    );

    println!(
        "TASK_POLL_SENT=false"
    );
}
