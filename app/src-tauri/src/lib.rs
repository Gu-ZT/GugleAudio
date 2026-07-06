use std::sync::Mutex;

use engine::{EngineController, EngineSnapshot};
use proto::{sample_graph, RouteEdge, RouteGraph, RouteValidationError};
use tauri::State;

struct AppState {
    engine: Mutex<EngineController>,
}

#[tauri::command]
fn get_route_graph() -> RouteGraph {
    sample_graph()
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState {
            engine: Mutex::new(EngineController::new()),
        })
        .invoke_handler(tauri::generate_handler![
            get_route_graph,
            validate_route_edge,
            start_engine,
            stop_engine,
            get_engine_snapshot,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
