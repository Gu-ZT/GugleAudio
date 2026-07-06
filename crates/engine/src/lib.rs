use anyhow::Result;
use proto::{sample_graph, RouteEdge, RouteGraph, RouteValidationError};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineState {
    Stopped,
    Starting,
    Running,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EngineSnapshot {
    pub state: EngineState,
    pub active_session: Option<String>,
    pub processed_frames: u64,
}

pub struct EngineController {
    graph: RouteGraph,
    state: EngineState,
    active_session: Option<String>,
    processed_frames: u64,
}

impl EngineController {
    pub fn new() -> Self {
        Self {
            graph: sample_graph(),
            state: EngineState::Stopped,
            active_session: None,
            processed_frames: 0,
        }
    }

    pub fn graph(&self) -> &RouteGraph {
        &self.graph
    }

    pub fn snapshot(&self) -> EngineSnapshot {
        EngineSnapshot {
            state: self.state,
            active_session: self.active_session.clone(),
            processed_frames: self.processed_frames,
        }
    }

    pub fn validate_edge(&self, edge: &RouteEdge) -> Result<(), RouteValidationError> {
        self.graph.validate_edge(edge)
    }

    pub fn start_loopback_session(&mut self) -> Result<EngineSnapshot> {
        self.state = EngineState::Starting;
        self.active_session = Some("default-loopback-session".into());
        self.processed_frames = 480;
        self.state = EngineState::Running;
        Ok(self.snapshot())
    }

    pub fn stop_session(&mut self) -> EngineSnapshot {
        self.state = EngineState::Stopped;
        self.active_session = None;
        self.snapshot()
    }
}

impl Default for EngineController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_and_stops_stub_session() {
        let mut controller = EngineController::new();
        let running = controller.start_loopback_session().unwrap();
        assert_eq!(running.state, EngineState::Running);
        assert_eq!(running.active_session.as_deref(), Some("default-loopback-session"));

        let stopped = controller.stop_session();
        assert_eq!(stopped.state, EngineState::Stopped);
        assert_eq!(stopped.active_session, None);
    }

    #[test]
    fn delegates_route_validation() {
        let controller = EngineController::new();
        let edge = RouteEdge {
            source_id: "network-output-game-pc".into(),
            target_id: "network-input-stream-pc".into(),
        };

        assert_eq!(
            controller.validate_edge(&edge),
            Err(RouteValidationError::NetworkToNetworkForbidden)
        );
    }
}
