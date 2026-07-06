use proto::{NodeDirection, RouteNode, TransportKind};
use serde::{Deserialize, Serialize};

pub type PeerId = String;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportMode {
    Pcm,
    Opus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkEndpoint {
    pub peer_id: PeerId,
    pub display_name: String,
    pub mode: TransportMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkState {
    Disconnected,
    Connecting,
    Connected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkStatus {
    pub endpoint: NetworkEndpoint,
    pub state: LinkState,
}

pub fn network_nodes_for_peer(endpoint: &NetworkEndpoint) -> [RouteNode; 2] {
    [
        RouteNode {
            id: format!("network-input-{}", endpoint.peer_id),
            name: format!("Network Input: {}", endpoint.display_name),
            transport: TransportKind::Network,
            direction: NodeDirection::Input,
        },
        RouteNode {
            id: format!("network-output-{}", endpoint.peer_id),
            name: format!("Network Output: {}", endpoint.display_name),
            transport: TransportKind::Network,
            direction: NodeDirection::Output,
        },
    ]
}
