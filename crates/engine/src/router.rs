use anyhow::{Context, Result};
use rtrb::{Consumer, Producer, RingBuffer};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use windows::Win32::Media::Audio::{
    IAudioCaptureClient, IAudioClient, IAudioRenderClient,
    IMMDeviceEnumerator, MMDeviceEnumerator, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_LOOPBACK,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoUninitialize, CLSCTX_ALL,
};

use crate::com_init_best_effort;

const RING_CAPACITY: usize = 48000 * 8 * 4 / 10; // ~100ms of 8ch 32-bit
const QUANTUM_MS: u64 = 5;

/// A route from one capture source to one render sink with a gain.
#[derive(Clone)]
pub struct ActiveRoute {
    pub source_device_id: String,
    pub sink_device_id: String,
    pub gain: f32,
}

/// Shared routing state that can be updated from the main thread.
pub struct RoutingTable {
    pub routes: Vec<ActiveRoute>,
    pub output_gains: HashMap<String, f32>,
}

/// The live audio router that manages capture/render threads.
pub struct AudioRouter {
    stop_flag: Arc<AtomicBool>,
    routing: Arc<Mutex<RoutingTable>>,
    peaks: Arc<Mutex<HashMap<String, f32>>>,
    frames_processed: Arc<AtomicU64>,
    mixer_thread: Option<thread::JoinHandle<()>>,
    capture_threads: Vec<thread::JoinHandle<()>>,
    render_threads: Vec<thread::JoinHandle<()>>,
}

struct CaptureHandle {
    device_id: String,
    consumer: Consumer<u8>,
}

struct RenderHandle {
    device_id: String,
    producer: Producer<u8>,
}

impl AudioRouter {
    /// Start the router with a set of source device IDs (with loopback flag) and sink device IDs.
    pub fn start(
        source_ids: Vec<(String, bool)>,
        sink_ids: Vec<String>,
        routes: Vec<ActiveRoute>,
        output_gains: HashMap<String, f32>,
    ) -> Result<Self> {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let frames_processed = Arc::new(AtomicU64::new(0));
        let routing = Arc::new(Mutex::new(RoutingTable { routes, output_gains }));
        let peaks = Arc::new(Mutex::new(HashMap::new()));

        let mut capture_consumers: Vec<CaptureHandle> = Vec::new();
        let mut capture_threads: Vec<thread::JoinHandle<()>> = Vec::new();

        // Start a capture thread for each source
        for (src_id, is_loopback) in &source_ids {
            let (producer, consumer) = RingBuffer::<u8>::new(RING_CAPACITY);
            capture_consumers.push(CaptureHandle {
                device_id: src_id.clone(),
                consumer,
            });

            let stop = stop_flag.clone();
            let frames = frames_processed.clone();
            let dev_id = src_id.clone();
            let loopback = *is_loopback;
            let handle = thread::Builder::new()
                .name(format!("cap-{}", &dev_id[..8.min(dev_id.len())]))
                .spawn(move || {
                    if let Err(e) = run_capture(&stop, &frames, producer, &dev_id, loopback) {
                        eprintln!("[capture {}] error: {e:#}", &dev_id[..8.min(dev_id.len())]);
                    }
                })
                .context("spawn capture thread")?;
            capture_threads.push(handle);
        }

        let mut render_producers: Vec<RenderHandle> = Vec::new();
        let mut render_threads: Vec<thread::JoinHandle<()>> = Vec::new();

        // Start a render thread for each sink
        for sink_id in &sink_ids {
            let (producer, consumer) = RingBuffer::<u8>::new(RING_CAPACITY);
            render_producers.push(RenderHandle {
                device_id: sink_id.clone(),
                producer,
            });

            let stop = stop_flag.clone();
            let dev_id = sink_id.clone();
            let handle = thread::Builder::new()
                .name(format!("ren-{}", &dev_id[..8.min(dev_id.len())]))
                .spawn(move || {
                    if let Err(e) = run_render(&stop, consumer, &dev_id) {
                        eprintln!("[render {}] error: {e:#}", &dev_id[..8.min(dev_id.len())]);
                    }
                })
                .context("spawn render thread")?;
            render_threads.push(handle);
        }

        // Start mixer thread
        let stop_mix = stop_flag.clone();
        let routing_mix = routing.clone();
        let peaks_mix = peaks.clone();
        let mixer_thread = thread::Builder::new()
            .name("mixer".into())
            .spawn(move || {
                run_mixer(&stop_mix, &routing_mix, &peaks_mix, capture_consumers, render_producers);
            })
            .context("spawn mixer thread")?;

        Ok(Self {
            stop_flag,
            routing,
            peaks,
            frames_processed,
            mixer_thread: Some(mixer_thread),
            capture_threads,
            render_threads,
        })
    }

    pub fn update_routes(&self, routes: Vec<ActiveRoute>, output_gains: HashMap<String, f32>) {
        if let Ok(mut rt) = self.routing.lock() {
            rt.routes = routes;
            rt.output_gains = output_gains;
        }
    }

    pub fn frames_processed(&self) -> u64 {
        self.frames_processed.load(Ordering::Relaxed)
    }

    pub fn get_peaks(&self) -> HashMap<String, f32> {
        self.peaks.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn stop(mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(h) = self.mixer_thread.take() { let _ = h.join(); }
        for h in self.capture_threads.drain(..) { let _ = h.join(); }
        for h in self.render_threads.drain(..) { let _ = h.join(); }
    }
}

impl Drop for AudioRouter {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}

// --- Mixer thread ---

fn run_mixer(
    stop: &AtomicBool,
    routing: &Mutex<RoutingTable>,
    peaks: &Mutex<HashMap<String, f32>>,
    mut captures: Vec<CaptureHandle>,
    mut renders: Vec<RenderHandle>,
) {
    let quantum_bytes = 4800;
    let mut src_bufs: HashMap<String, Vec<u8>> = HashMap::new();

    for cap in &captures {
        src_bufs.insert(cap.device_id.clone(), vec![0u8; quantum_bytes]);
    }

    while !stop.load(Ordering::Relaxed) {
        thread::sleep(std::time::Duration::from_millis(QUANTUM_MS));

        // Read from each capture ring
        for cap in captures.iter_mut() {
            let buf = src_bufs.get_mut(&cap.device_id).unwrap();
            let avail = cap.consumer.slots();
            let to_read = avail.min(quantum_bytes);
            if to_read > 0 {
                let chunk = cap.consumer.read_chunk(to_read).unwrap();
                let (a, b) = chunk.as_slices();
                buf[..a.len()].copy_from_slice(a);
                if !b.is_empty() {
                    buf[a.len()..a.len() + b.len()].copy_from_slice(b);
                }
                let total = a.len() + b.len();
                for byte in buf[total..quantum_bytes].iter_mut() { *byte = 0; }
                chunk.commit_all();
            } else {
                buf.fill(0);
            }
        }

        // Compute peaks per source (interpret as f32 samples)
        let mut new_peaks: HashMap<String, f32> = HashMap::new();
        for (id, buf) in &src_bufs {
            let samples = unsafe {
                std::slice::from_raw_parts(buf.as_ptr() as *const f32, buf.len() / 4)
            };
            let peak = samples.iter().fold(0.0f32, |max, &s| max.max(s.abs()));
            new_peaks.insert(id.clone(), peak.min(1.0));
        }
        if let Ok(mut p) = peaks.lock() {
            *p = new_peaks;
        }

        // Route: for each active route, write source bytes to the corresponding sink's ring
        let rt = routing.lock().unwrap();
        for route in &rt.routes {
            let Some(src_bytes) = src_bufs.get(&route.source_device_id) else { continue };
            let Some(ren) = renders.iter_mut().find(|r| r.device_id == route.sink_device_id) else { continue };

            let writable = ren.producer.slots();
            let to_write = quantum_bytes.min(writable);
            if to_write > 0 {
                ren.producer
                    .write_chunk_uninit(to_write)
                    .unwrap()
                    .fill_from_iter(src_bytes[..to_write].iter().copied());
            }
        }
        drop(rt);
    }
}

// --- Capture thread: opens a specific device by ID in loopback mode ---

fn run_capture(
    stop: &AtomicBool,
    frames_processed: &AtomicU64,
    mut producer: Producer<u8>,
    device_id: &str,
    is_loopback: bool,
) -> Result<()> {
    unsafe {
        let needs_uninit = com_init_best_effort();

        let result = (|| -> Result<()> {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;

            let wide_id: Vec<u16> = device_id.encode_utf16().chain(std::iter::once(0)).collect();
            let device = enumerator.GetDevice(windows::core::PCWSTR(wide_id.as_ptr()))?;

            let audio_client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
            let mix_format = audio_client.GetMixFormat()?;

            let flags = if is_loopback {
                AUDCLNT_STREAMFLAGS_LOOPBACK
            } else {
                AUDCLNT_STREAMFLAGS_LOOPBACK & !AUDCLNT_STREAMFLAGS_LOOPBACK // 0
            };
            audio_client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 200_000, 0, mix_format, None)?;

            let capture_client: IAudioCaptureClient = audio_client.GetService()?;
            audio_client.Start()?;

            let ch = { (*mix_format).nChannels };
            let bits = { (*mix_format).wBitsPerSample };
            let rate = { (*mix_format).nSamplesPerSec };
            eprintln!("[capture] started device={} loopback={} format={}ch/{}bit/{}Hz",
                &device_id[..8.min(device_id.len())], is_loopback, ch, bits, rate);

            while !stop.load(Ordering::Relaxed) {
                thread::sleep(std::time::Duration::from_millis(5));

                let mut pkt = capture_client.GetNextPacketSize()?;
                while pkt > 0 {
                    let mut buf = std::ptr::null_mut();
                    let mut nframes = 0u32;
                    let mut flags_out = 0u32;
                    capture_client.GetBuffer(&mut buf, &mut nframes, &mut flags_out, None, None)?;

                    let frame_size = (*mix_format).nBlockAlign as usize;
                    let byte_count = nframes as usize * frame_size;
                    let data = std::slice::from_raw_parts(buf as *const u8, byte_count);

                    let writable = producer.slots();
                    let to_write = byte_count.min(writable);
                    if to_write > 0 {
                        producer
                            .write_chunk_uninit(to_write)
                            .unwrap()
                            .fill_from_iter(data[..to_write].iter().copied());
                    }

                    frames_processed.fetch_add(nframes as u64, Ordering::Relaxed);
                    capture_client.ReleaseBuffer(nframes)?;
                    pkt = capture_client.GetNextPacketSize()?;
                }
            }

            audio_client.Stop()?;
            Ok(())
        })();

        if needs_uninit { CoUninitialize(); }
        result
    }
}

// --- Render thread: opens a specific device by ID ---

fn run_render(stop: &AtomicBool, mut consumer: Consumer<u8>, device_id: &str) -> Result<()> {
    unsafe {
        let needs_uninit = com_init_best_effort();

        let result = (|| -> Result<()> {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;

            let wide_id: Vec<u16> = device_id.encode_utf16().chain(std::iter::once(0)).collect();
            let device = enumerator.GetDevice(windows::core::PCWSTR(wide_id.as_ptr()))?;

            let audio_client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
            let mix_format = audio_client.GetMixFormat()?;

            audio_client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                windows::Win32::Media::Audio::AUDCLNT_STREAMFLAGS_NOPERSIST,
                200_000,
                0,
                mix_format,
                None,
            )?;

            let buffer_size = audio_client.GetBufferSize()?;
            let render_client: IAudioRenderClient = audio_client.GetService()?;
            let frame_size = (*mix_format).nBlockAlign as usize;

            audio_client.Start()?;

            let ch = { (*mix_format).nChannels };
            let bits = { (*mix_format).wBitsPerSample };
            let rate = { (*mix_format).nSamplesPerSec };
            eprintln!("[render] started device={} format={}ch/{}bit/{}Hz",
                &device_id[..8.min(device_id.len())], ch, bits, rate);

            while !stop.load(Ordering::Relaxed) {
                thread::sleep(std::time::Duration::from_millis(5));

                let padding = audio_client.GetCurrentPadding()?;
                let available_frames = buffer_size - padding;
                if available_frames == 0 { continue; }

                let available_bytes = available_frames as usize * frame_size;
                let readable = consumer.slots();
                let frames_to_write = if readable >= available_bytes {
                    available_frames
                } else {
                    (readable / frame_size) as u32
                };

                if frames_to_write == 0 {
                    let buffer = render_client.GetBuffer(available_frames)?;
                    std::ptr::write_bytes(buffer, 0, available_frames as usize * frame_size);
                    render_client.ReleaseBuffer(available_frames, 0)?;
                    continue;
                }

                let byte_count = frames_to_write as usize * frame_size;
                let buffer = render_client.GetBuffer(frames_to_write)?;
                let out_slice = std::slice::from_raw_parts_mut(buffer, byte_count);

                let chunk = consumer.read_chunk(byte_count).unwrap();
                let (first, second) = chunk.as_slices();
                out_slice[..first.len()].copy_from_slice(first);
                if !second.is_empty() {
                    out_slice[first.len()..first.len() + second.len()].copy_from_slice(second);
                }
                chunk.commit_all();

                render_client.ReleaseBuffer(frames_to_write, 0)?;
            }

            audio_client.Stop()?;
            Ok(())
        })();

        if needs_uninit { CoUninitialize(); }
        result
    }
}
