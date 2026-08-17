mod adapters;
pub mod core;
pub mod runtime;

use crate::core::NodeState;

#[tauri::command]
fn get_node_state() -> NodeState {
    NodeState::detect()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![get_node_state])
        .run(tauri::generate_context!())
        .expect("error while running EdgeSwarm Unified Node");
}
