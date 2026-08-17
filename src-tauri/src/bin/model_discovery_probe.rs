use edgeswarm_unified_node_lib::core::model_discovery::discover_models;
use std::path::Path;

fn main() {
    let root = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/home/jeff/swarm-data/models".into());

    let models = discover_models(Path::new(&root));

    println!("MODEL_ROOT={root}");
    println!("DISCOVERED_MODEL_COUNT={}", models.len());

    for model in models {
        println!(
            "{} | capability={} | tier={} | runtime={} | ctx={} | max_tokens={} | experimental={} | file={}",
            model.selected_model,
            model.capability,
            model.tier,
            model.runtime,
            model.default_ctx,
            model.default_max_tokens,
            model.experimental,
            model.file_name
        );
    }
}
