use std::collections::HashMap;
use std::sync::Mutex;

use engine::{AudioDeviceInfo, EngineController, EngineSnapshot};
use proto::{RouteEdge, RouteGraph, RouteValidationError};
use tauri::State;

struct AppState {
    engine: Mutex<EngineController>,
}

#[tauri::command]
fn get_route_graph(state: State<'_, AppState>) -> RouteGraph {
    let engine = state.engine.lock().expect("engine mutex poisoned");
    engine.graph().clone()
}

#[tauri::command]
fn get_audio_devices(state: State<'_, AppState>) -> Vec<AudioDeviceInfo> {
    let engine = state.engine.lock().expect("engine mutex poisoned");
    engine.all_devices().to_vec()
}

#[tauri::command]
fn validate_route_edge(
    state: State<'_, AppState>,
    edge: RouteEdge,
) -> Result<(), RouteValidationError> {
    let engine = state.engine.lock().expect("engine mutex poisoned");
    engine.validate_edge(&edge)
}

#[tauri::command]
fn add_route(state: State<'_, AppState>, edge: RouteEdge) -> Result<RouteGraph, RouteValidationError> {
    let mut engine = state.engine.lock().expect("engine mutex poisoned");
    engine.add_edge(edge)?;
    Ok(engine.graph().clone())
}

#[tauri::command]
fn remove_route(state: State<'_, AppState>, source_id: String, target_id: String) -> RouteGraph {
    let mut engine = state.engine.lock().expect("engine mutex poisoned");
    engine.remove_edge(&source_id, &target_id);
    engine.graph().clone()
}

#[tauri::command]
fn set_volume(state: State<'_, AppState>, key: String, gain: f32) {
    let mut engine = state.engine.lock().expect("engine mutex poisoned");
    engine.set_volume(key, gain);
}

#[tauri::command]
fn get_engine_snapshot(state: State<'_, AppState>) -> EngineSnapshot {
    let engine = state.engine.lock().expect("engine mutex poisoned");
    engine.snapshot()
}

#[tauri::command]
fn refresh_audio_devices(state: State<'_, AppState>) -> RouteGraph {
    let mut engine = state.engine.lock().expect("engine mutex poisoned");
    engine.refresh_audio_devices();
    engine.graph().clone()
}

#[tauri::command]
fn set_monitored_inputs(state: State<'_, AppState>, node_ids: Vec<String>) {
    let mut engine = state.engine.lock().expect("engine mutex poisoned");
    engine.set_monitored_inputs(node_ids);
}

#[tauri::command]
fn get_peaks(state: State<'_, AppState>) -> HashMap<String, f32> {
    let engine = state.engine.lock().expect("engine mutex poisoned");
    engine.get_peaks()
}

#[tauri::command]
fn stop_engine(state: State<'_, AppState>) {
    let mut engine = state.engine.lock().expect("engine mutex poisoned");
    engine.stop_engine();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState {
            engine: Mutex::new(EngineController::new()),
        })
        .invoke_handler(tauri::generate_handler![
            get_route_graph,
            get_audio_devices,
            validate_route_edge,
            add_route,
            remove_route,
            set_volume,
            set_monitored_inputs,
            get_engine_snapshot,
            get_peaks,
            refresh_audio_devices,
            stop_engine,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
