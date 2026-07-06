pub mod router;

use anyhow::{Context, Result};
use proto::{NodeDirection, RouteEdge, RouteGraph, RouteNode, RouteValidationError, TransportKind};
use router::{ActiveRoute, AudioRouter};
use serde::Serialize;
use std::collections::HashMap;
use windows::core::PWSTR;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{
    eCapture, eConsole, eRender, IMMDevice, IMMDeviceCollection, IMMDeviceEnumerator,
    MMDeviceEnumerator, DEVICE_STATE_ACTIVE, EDataFlow,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineState {
    Stopped,
    Running,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AudioDeviceInfo {
    pub id: String,
    pub name: String,
    pub flow: String,
    pub role: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EngineSnapshot {
    pub state: EngineState,
    pub active_routes: usize,
    pub processed_frames: u64,
}

pub struct EngineController {
    graph: RouteGraph,
    state: EngineState,
    all_devices: Vec<AudioDeviceInfo>,
    router: Option<AudioRouter>,
    volumes: HashMap<String, f32>,       // edge key "src>tgt" -> gain 0.0..1.0
    output_volumes: HashMap<String, f32>, // device id -> gain
}

impl EngineController {
    pub fn new() -> Self {
        let all_devices = enumerate_all_devices().unwrap_or_default();
        let graph = build_graph_from_devices(&all_devices);

        Self {
            graph,
            state: EngineState::Stopped,
            all_devices,
            router: None,
            volumes: HashMap::new(),
            output_volumes: HashMap::new(),
        }
    }

    pub fn graph(&self) -> &RouteGraph {
        &self.graph
    }

    pub fn all_devices(&self) -> &[AudioDeviceInfo] {
        &self.all_devices
    }

    pub fn snapshot(&self) -> EngineSnapshot {
        EngineSnapshot {
            state: self.state,
            active_routes: self.graph.edges.len(),
            processed_frames: self.router.as_ref().map(|r| r.frames_processed()).unwrap_or(0),
        }
    }

    pub fn validate_edge(&self, edge: &RouteEdge) -> Result<(), RouteValidationError> {
        self.graph.validate_edge(edge)
    }

    pub fn refresh_audio_devices(&mut self) {
        self.all_devices = enumerate_all_devices().unwrap_or_default();
        let old_edges = std::mem::take(&mut self.graph.edges);
        self.graph = build_graph_from_devices(&self.all_devices);
        for edge in old_edges {
            if self.graph.validate_edge(&edge).is_ok() {
                self.graph.edges.push(edge);
            }
        }
    }

    pub fn add_edge(&mut self, edge: RouteEdge) -> Result<(), RouteValidationError> {
        self.graph.validate_edge(&edge)?;
        if !self.graph.edges.iter().any(|e| e.source_id == edge.source_id && e.target_id == edge.target_id) {
            self.graph.edges.push(edge);
            self.apply_routing();
        }
        Ok(())
    }

    pub fn remove_edge(&mut self, source_id: &str, target_id: &str) {
        let had = self.graph.edges.len();
        self.graph.edges.retain(|e| !(e.source_id == source_id && e.target_id == target_id));
        if self.graph.edges.len() != had {
            self.apply_routing();
        }
    }

    pub fn set_volume(&mut self, key: String, gain: f32) {
        if key.starts_with("out-") {
            let device_id = key.strip_prefix("out-").unwrap_or(&key).to_string();
            self.output_volumes.insert(device_id, gain);
        } else {
            self.volumes.insert(key, gain);
        }
        // Hot-update routing table without restarting threads
        if let Some(router) = &self.router {
            let (routes, output_gains) = self.build_active_routes();
            router.update_routes(routes, output_gains);
        }
    }

    pub fn stop_engine(&mut self) {
        if let Some(router) = self.router.take() {
            router.stop();
        }
        self.state = EngineState::Stopped;
    }

    /// Rebuild and (re)start the audio router based on current edges.
    fn apply_routing(&mut self) {
        // Stop existing router
        if let Some(router) = self.router.take() {
            router.stop();
        }

        if self.graph.edges.is_empty() {
            self.state = EngineState::Stopped;
            return;
        }

        // Collect unique source and sink device IDs from edges
        // source nodes in graph have direction=Output, their id is "device-{wasapi_id}"
        let mut source_ids: Vec<String> = Vec::new();
        let mut sink_ids: Vec<String> = Vec::new();

        for edge in &self.graph.edges {
            let src_dev = self.node_to_device_id(&edge.source_id);
            let tgt_dev = self.node_to_device_id(&edge.target_id);
            if let Some(s) = src_dev {
                if !source_ids.contains(&s) { source_ids.push(s); }
            }
            if let Some(t) = tgt_dev {
                if !sink_ids.contains(&t) { sink_ids.push(t); }
            }
        }

        if source_ids.is_empty() || sink_ids.is_empty() {
            self.state = EngineState::Stopped;
            return;
        }

        let (routes, output_gains) = self.build_active_routes();

        match AudioRouter::start(source_ids, sink_ids, routes, output_gains) {
            Ok(router) => {
                self.router = Some(router);
                self.state = EngineState::Running;
            }
            Err(e) => {
                eprintln!("[engine] failed to start router: {e:#}");
                self.state = EngineState::Stopped;
            }
        }
    }

    fn build_active_routes(&self) -> (Vec<ActiveRoute>, HashMap<String, f32>) {
        let routes: Vec<ActiveRoute> = self.graph.edges.iter().filter_map(|edge| {
            let src = self.node_to_device_id(&edge.source_id)?;
            let tgt = self.node_to_device_id(&edge.target_id)?;
            let key = format!("{}>{}", edge.source_id, edge.target_id);
            let gain = self.volumes.get(&key).copied().unwrap_or(1.0);
            Some(ActiveRoute { source_device_id: src, sink_device_id: tgt, gain })
        }).collect();

        let output_gains: HashMap<String, f32> = self.output_volumes.clone();
        (routes, output_gains)
    }

    /// Convert a graph node ID like "device-{wasapi_id}" to the raw WASAPI device ID.
    fn node_to_device_id(&self, node_id: &str) -> Option<String> {
        node_id.strip_prefix("device-").map(|s| s.to_string())
    }
}

impl Default for EngineController {
    fn default() -> Self {
        Self::new()
    }
}

// --- WASAPI helpers ---

pub fn com_init_best_effort() -> bool {
    unsafe {
        let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
        hr.is_ok()
    }
}

fn enumerate_all_devices() -> Result<Vec<AudioDeviceInfo>> {
    unsafe {
        let needs_uninit = com_init_best_effort();

        let result = (|| {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let mut devices = Vec::new();
            enumerate_flow(&enumerator, eRender, "render", &mut devices)?;
            enumerate_flow(&enumerator, eCapture, "capture", &mut devices)?;
            Ok(devices)
        })();

        if needs_uninit { CoUninitialize(); }
        result
    }
}

fn enumerate_flow(
    enumerator: &IMMDeviceEnumerator,
    flow: EDataFlow,
    flow_name: &str,
    out: &mut Vec<AudioDeviceInfo>,
) -> Result<()> {
    unsafe {
        let collection: IMMDeviceCollection =
            enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)?;
        let count = collection.GetCount()?;

        let default_device = enumerator
            .GetDefaultAudioEndpoint(flow, eConsole)
            .ok()
            .and_then(|d| d.GetId().ok())
            .and_then(|id| pwstr_to_string(id).ok());

        for i in 0..count {
            let device = collection.Item(i)?;
            let id = pwstr_to_string(device.GetId()?)?;
            let name = get_device_friendly_name(&device).unwrap_or_else(|_| id.clone());
            let is_default = default_device.as_deref() == Some(&id);

            out.push(AudioDeviceInfo {
                id,
                name,
                flow: flow_name.into(),
                role: if is_default { "default".into() } else { "normal".into() },
            });
        }
    }
    Ok(())
}

fn build_graph_from_devices(devices: &[AudioDeviceInfo]) -> RouteGraph {
    let mut nodes: Vec<RouteNode> = devices
        .iter()
        .map(|d| {
            let direction = match d.flow.as_str() {
                "render" => NodeDirection::Input,
                _ => NodeDirection::Output,
            };
            RouteNode {
                id: format!("device-{}", d.id),
                name: d.name.clone(),
                transport: TransportKind::Local,
                direction,
            }
        })
        .collect();

    nodes.push(RouteNode {
        id: "network-input-stream-pc".into(),
        name: "Network: Stream PC".into(),
        transport: TransportKind::Network,
        direction: NodeDirection::Input,
    });
    nodes.push(RouteNode {
        id: "network-output-game-pc".into(),
        name: "Network: Game PC".into(),
        transport: TransportKind::Network,
        direction: NodeDirection::Output,
    });

    RouteGraph { nodes, edges: vec![] }
}

fn get_device_friendly_name(device: &IMMDevice) -> Result<String> {
    unsafe {
        let store: IPropertyStore = device.OpenPropertyStore(STGM_READ)?;
        let prop = store.GetValue(&PKEY_Device_FriendlyName)?;
        let wide = prop.Anonymous.Anonymous.Anonymous.pwszVal;
        if wide.0.is_null() {
            anyhow::bail!("friendly name is null");
        }
        wide.to_string().context("wide string conversion failed")
    }
}

fn pwstr_to_string(value: PWSTR) -> Result<String> {
    unsafe { value.to_string().context("wide string conversion failed") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::sample_graph;

    #[test]
    fn snapshot_without_router() {
        let controller = EngineController {
            graph: sample_graph(),
            state: EngineState::Stopped,
            all_devices: vec![],
            router: None,
            volumes: HashMap::new(),
            output_volumes: HashMap::new(),
        };
        let snap = controller.snapshot();
        assert_eq!(snap.state, EngineState::Stopped);
        assert_eq!(snap.active_routes, 0);
    }

    #[test]
    fn delegates_route_validation() {
        let controller = EngineController {
            graph: sample_graph(),
            state: EngineState::Stopped,
            all_devices: vec![],
            router: None,
            volumes: HashMap::new(),
            output_volumes: HashMap::new(),
        };
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
