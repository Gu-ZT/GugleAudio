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
fn start_engine(state: State<'_, AppState>) -> Result<EngineSnapshot, String> {
    let mut engine = state.engine.lock().map_err(|_| "engine mutex poisoned".to_string())?;
    engine
        .start_loopback_session()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn stop_engine(state: State<'_, AppState>) -> Result<EngineSnapshot, String> {
    let mut engine = state.engine.lock().map_err(|_| "engine mutex poisoned".to_string())?;
    Ok(engine.stop_session())
}

#[tauri::command]
fn get_engine_snapshot(state: State<'_, AppState>) -> Result<EngineSnapshot, String> {
    let engine = state.engine.lock().map_err(|_| "engine mutex poisoned".to_string())?;
    Ok(engine.snapshot())
}

#[tauri::command]
fn refresh_audio_devices(state: State<'_, AppState>) -> Result<RouteGraph, String> {
    let mut engine = state.engine.lock().map_err(|_| "engine mutex poisoned".to_string())?;
    engine.refresh_audio_devices();
    Ok(engine.graph().clone())
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
            start_engine,
            stop_engine,
            get_engine_snapshot,
            refresh_audio_devices,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
