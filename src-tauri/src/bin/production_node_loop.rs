use edgeswarm_unified_node_lib::core::node_service::run_node_service;
use std::{
    env,
    sync::{
        atomic::AtomicBool,
        Arc,
    },
};
use zeroize::Zeroizing;

fn main() {
    let execute = env::var("EXECUTE")
        .map(|value| {
            value == "1" ||
                value.eq_ignore_ascii_case("true")
        })
        .unwrap_or(false);

    if !execute {
        println!("EXECUTE=false");
        println!("RUNNER_MODE=continuous_limit_1");
        println!("REAL_WALLET_UNLOCKED=false");
        println!("NETWORK_REQUEST_SENT=false");
        println!("HEARTBEAT_SENT=false");
        println!("TASK_POLL_SENT=false");
        println!("TASK_CLAIMED=false");
        println!("RESULT_SUBMITTED=false");
        return;
    }

    let password = match rpassword::prompt_password(
        "Wallet password: ",
    ) {
        Ok(value) => Zeroizing::new(value),
        Err(_) => {
            println!(
                "PRODUCTION_TASK_RUNNER_ERROR=wallet_password_read_failed"
            );
            std::process::exit(1);
        }
    };

    let stop = Arc::new(AtomicBool::new(false));

    if let Err(error) = run_node_service(stop, password) {
        println!(
            "PRODUCTION_TASK_RUNNER_ERROR={}",
            error.replace('\n', " ")
        );
        std::process::exit(1);
    }
}
