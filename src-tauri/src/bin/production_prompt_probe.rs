use edgeswarm_unified_node_lib::core::{
    certification_workload::built_in_3b_realworld_v2,
    production_prompt::compile_certification_prompt,
};

fn main() {
    let pack = built_in_3b_realworld_v2().unwrap();
    let workload = &pack.workloads[0];
    let compiled = compile_certification_prompt(workload).unwrap();

    println!("WORKLOAD_ID={}", workload.id);
    println!("REQUIRED_MODEL={}", compiled.required_model);
    println!("ADAPTER_LANE={}", compiled.adapter_lane);
    println!("POLICY_VERSION={}", compiled.policy_version);
    println!("SYSTEM={}", compiled.system_text);

    println!();
    println!("=== COMPILED USER PROMPT START ===");

    for line in compiled.user_text.lines().take(40) {
        println!("{line}");
    }

    println!("=== COMPILED USER PROMPT PREVIEW END ===");
}
