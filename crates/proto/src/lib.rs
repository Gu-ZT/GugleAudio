use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportKind {
    Local,
    Virtual,
    Network,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeDirection {
    Input,
    Output,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteNode {
    pub id: String,
    pub name: String,
    pub transport: TransportKind,
    pub direction: NodeDirection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteEdge {
    pub source_id: String,
    pub target_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteGraph {
    pub nodes: Vec<RouteNode>,
    pub edges: Vec<RouteEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
#[serde(tag = "code", content = "detail", rename_all = "snake_case")]
pub enum RouteValidationError {
    #[error("source node was not found")]
    SourceNotFound(String),
    #[error("target node was not found")]
    TargetNotFound(String),
    #[error("routes must connect an output to an input")]
    DirectionMismatch,
    #[error("network nodes cannot connect directly to network nodes")]
    NetworkToNetworkForbidden,
    #[error("self-routing is not allowed")]
    SelfRouteForbidden,
}

impl RouteGraph {
    pub fn validate_edge(&self, edge: &RouteEdge) -> Result<(), RouteValidationError> {
        if edge.source_id == edge.target_id {
            return Err(RouteValidationError::SelfRouteForbidden);
        }

        let source = self
            .nodes
            .iter()
            .find(|node| node.id == edge.source_id)
            .ok_or_else(|| RouteValidationError::SourceNotFound(edge.source_id.clone()))?;
        let target = self
            .nodes
            .iter()
            .find(|node| node.id == edge.target_id)
            .ok_or_else(|| RouteValidationError::TargetNotFound(edge.target_id.clone()))?;

        if source.direction != NodeDirection::Output || target.direction != NodeDirection::Input {
            return Err(RouteValidationError::DirectionMismatch);
        }

        if source.transport == TransportKind::Network && target.transport == TransportKind::Network {
            return Err(RouteValidationError::NetworkToNetworkForbidden);
        }

        Ok(())
    }
}

pub fn sample_graph() -> RouteGraph {
    RouteGraph {
        nodes: vec![
            RouteNode {
                id: "local-output-speakers".into(),
                name: "Speakers".into(),
                transport: TransportKind::Local,
                direction: NodeDirection::Input,
            },
            RouteNode {
                id: "local-input-loopback".into(),
                name: "Desktop Loopback".into(),
                transport: TransportKind::Local,
                direction: NodeDirection::Output,
            },
            RouteNode {
                id: "virtual-input-cable-a".into(),
                name: "Gugle Cable A In".into(),
                transport: TransportKind::Virtual,
                direction: NodeDirection::Input,
            },
            RouteNode {
                id: "virtual-output-cable-a".into(),
                name: "Gugle Cable A Out".into(),
                transport: TransportKind::Virtual,
                direction: NodeDirection::Output,
            },
            RouteNode {
                id: "network-input-stream-pc".into(),
                name: "Network Input: Stream PC".into(),
                transport: TransportKind::Network,
                direction: NodeDirection::Input,
            },
            RouteNode {
                id: "network-output-game-pc".into(),
                name: "Network Output: Game PC".into(),
                transport: TransportKind::Network,
                direction: NodeDirection::Output,
            },
        ],
        edges: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_local_to_network() {
        let graph = sample_graph();
        let edge = RouteEdge {
            source_id: "local-input-loopback".into(),
            target_id: "network-input-stream-pc".into(),
        };

        assert_eq!(graph.validate_edge(&edge), Ok(()));
    }

    #[test]
    fn rejects_network_to_network() {
        let graph = sample_graph();
        let edge = RouteEdge {
            source_id: "network-output-game-pc".into(),
            target_id: "network-input-stream-pc".into(),
        };

        assert_eq!(
            graph.validate_edge(&edge),
            Err(RouteValidationError::NetworkToNetworkForbidden)
        );
    }

    #[test]
    fn rejects_wrong_direction() {
        let graph = sample_graph();
        let edge = RouteEdge {
            source_id: "virtual-input-cable-a".into(),
            target_id: "network-input-stream-pc".into(),
        };

        assert_eq!(graph.validate_edge(&edge), Err(RouteValidationError::DirectionMismatch));
    }
}
