use anyhow::{Context, Result};
use proto::{NodeDirection, RouteEdge, RouteGraph, RouteNode, RouteValidationError, TransportKind};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use windows::core::PWSTR;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{
    eCapture, eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDevice,
    IMMDeviceCollection, IMMDeviceEnumerator, MMDeviceEnumerator, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_LOOPBACK, DEVICE_STATE_ACTIVE, EDataFlow,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineState {
    Stopped,
    Starting,
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
    pub active_session: Option<String>,
    pub processed_frames: u64,
    pub default_render_device: Option<AudioDeviceInfo>,
}

struct LoopbackSession {
    stop_flag: Arc<AtomicBool>,
    frames_captured: Arc<AtomicU64>,
    thread_handle: Option<thread::JoinHandle<()>>,
}

pub struct EngineController {
    graph: RouteGraph,
    state: EngineState,
    default_render_device: Option<AudioDeviceInfo>,
    session: Option<LoopbackSession>,
    all_devices: Vec<AudioDeviceInfo>,
}

impl EngineController {
    pub fn new() -> Self {
        let all_devices = enumerate_all_devices().unwrap_or_default();
        let default_render_device = all_devices
            .iter()
            .find(|d| d.flow == "render" && d.role == "default")
            .or_else(|| all_devices.iter().find(|d| d.flow == "render"))
            .cloned();

        let graph = build_graph_from_devices(&all_devices);

        Self {
            graph,
            state: EngineState::Stopped,
            default_render_device,
            session: None,
            all_devices,
        }
    }

    pub fn graph(&self) -> &RouteGraph {
        &self.graph
    }

    pub fn all_devices(&self) -> &[AudioDeviceInfo] {
        &self.all_devices
    }

    pub fn snapshot(&self) -> EngineSnapshot {
        let processed_frames = self
            .session
            .as_ref()
            .map(|s| s.frames_captured.load(Ordering::Relaxed))
            .unwrap_or(0);

        EngineSnapshot {
            state: self.state,
            active_session: if self.session.is_some() {
                Some("loopback-capture".into())
            } else {
                None
            },
            processed_frames,
            default_render_device: self.default_render_device.clone(),
        }
    }

    pub fn validate_edge(&self, edge: &RouteEdge) -> Result<(), RouteValidationError> {
        self.graph.validate_edge(edge)
    }

    pub fn refresh_audio_devices(&mut self) {
        self.all_devices = enumerate_all_devices().unwrap_or_default();
        self.default_render_device = self
            .all_devices
            .iter()
            .find(|d| d.flow == "render" && d.role == "default")
            .or_else(|| self.all_devices.iter().find(|d| d.flow == "render"))
            .cloned();
        self.graph = build_graph_from_devices(&self.all_devices);
    }

    pub fn add_edge(&mut self, edge: RouteEdge) -> Result<(), RouteValidationError> {
        self.graph.validate_edge(&edge)?;
        if !self.graph.edges.iter().any(|e| e.source_id == edge.source_id && e.target_id == edge.target_id) {
            self.graph.edges.push(edge);
        }
        Ok(())
    }

    pub fn remove_edge(&mut self, source_id: &str, target_id: &str) {
        self.graph.edges.retain(|e| !(e.source_id == source_id && e.target_id == target_id));
    }

    pub fn start_loopback_session(&mut self) -> Result<EngineSnapshot> {
        if self.session.is_some() {
            anyhow::bail!("session already running");
        }

        self.refresh_audio_devices();
        self.state = EngineState::Starting;

        let stop_flag = Arc::new(AtomicBool::new(false));
        let frames_captured = Arc::new(AtomicU64::new(0));

        let stop = stop_flag.clone();
        let frames = frames_captured.clone();

        let thread_handle = thread::Builder::new()
            .name("loopback-capture".into())
            .spawn(move || {
                if let Err(e) = loopback_capture_thread(&stop, &frames) {
                    eprintln!("[loopback-capture] error: {e:#}");
                }
            })
            .context("failed to spawn loopback capture thread")?;

        self.session = Some(LoopbackSession {
            stop_flag,
            frames_captured,
            thread_handle: Some(thread_handle),
        });
        self.state = EngineState::Running;
        Ok(self.snapshot())
    }

    pub fn stop_session(&mut self) -> EngineSnapshot {
        if let Some(mut session) = self.session.take() {
            session.stop_flag.store(true, Ordering::Relaxed);
            if let Some(handle) = session.thread_handle.take() {
                let _ = handle.join();
            }
        }
        self.state = EngineState::Stopped;
        self.snapshot()
    }
}

impl Default for EngineController {
    fn default() -> Self {
        Self::new()
    }
}

// --- WASAPI helpers ---

fn enumerate_all_devices() -> Result<Vec<AudioDeviceInfo>> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .context("CoInitializeEx failed")?;

        let result = (|| {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;

            let mut devices = Vec::new();
            enumerate_flow(&enumerator, eRender, "render", &mut devices)?;
            enumerate_flow(&enumerator, eCapture, "capture", &mut devices)?;
            Ok(devices)
        })();

        CoUninitialize();
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

    RouteGraph {
        nodes,
        edges: vec![],
    }
}

fn get_device_friendly_name(device: &IMMDevice) -> Result<String> {
    unsafe {
        let store: IPropertyStore = device.OpenPropertyStore(STGM_READ)?;
        let prop = store.GetValue(&PKEY_Device_FriendlyName)?;
        let wide = prop
            .Anonymous
            .Anonymous
            .Anonymous
            .pwszVal;
        if wide.0.is_null() {
            anyhow::bail!("friendly name is null");
        }
        wide.to_string().context("wide string conversion failed")
    }
}

fn loopback_capture_thread(stop: &AtomicBool, frames_captured: &AtomicU64) -> Result<()> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .context("CoInitializeEx failed in capture thread")?;

        let result = run_loopback_capture(stop, frames_captured);
        CoUninitialize();
        result
    }
}

fn run_loopback_capture(stop: &AtomicBool, frames_captured: &AtomicU64) -> Result<()> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;

        let audio_client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;

        let mix_format = audio_client.GetMixFormat()?;
        audio_client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK,
            10_000_000, // 1 second buffer (100ns units)
            0,
            mix_format,
            None,
        )?;

        let capture_client: IAudioCaptureClient = audio_client.GetService()?;
        audio_client.Start()?;

        while !stop.load(Ordering::Relaxed) {
            thread::sleep(std::time::Duration::from_millis(10));

            let mut packet_length = capture_client.GetNextPacketSize()?;
            while packet_length > 0 {
                let mut buffer = std::ptr::null_mut();
                let mut num_frames = 0u32;
                let mut flags = 0u32;
                capture_client.GetBuffer(
                    &mut buffer,
                    &mut num_frames,
                    &mut flags,
                    None,
                    None,
                )?;

                frames_captured.fetch_add(num_frames as u64, Ordering::Relaxed);
                capture_client.ReleaseBuffer(num_frames)?;
                packet_length = capture_client.GetNextPacketSize()?;
            }
        }

        audio_client.Stop()?;
    }
    Ok(())
}

fn pwstr_to_string(value: PWSTR) -> Result<String> {
    unsafe { value.to_string().context("wide string conversion failed") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::sample_graph;

    #[test]
    fn snapshot_without_session() {
        let controller = EngineController {
            graph: sample_graph(),
            state: EngineState::Stopped,
            default_render_device: None,
            session: None,
            all_devices: vec![],
        };
        let snap = controller.snapshot();
        assert_eq!(snap.state, EngineState::Stopped);
        assert_eq!(snap.active_session, None);
        assert_eq!(snap.processed_frames, 0);
    }

    #[test]
    fn delegates_route_validation() {
        let controller = EngineController {
            graph: sample_graph(),
            state: EngineState::Stopped,
            default_render_device: None,
            session: None,
            all_devices: vec![],
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
