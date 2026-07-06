use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use windows::Win32::Media::Audio::{
    IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator,
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
};
use windows::Win32::System::Com::{CoCreateInstance, CoUninitialize, CLSCTX_ALL};

use crate::com_init_best_effort;

/// Lightweight metering engine: captures audio from devices just to compute peak levels.
pub struct MeterEngine {
    stop_flag: Arc<AtomicBool>,
    peaks: Arc<Mutex<HashMap<String, f32>>>,
    threads: Vec<thread::JoinHandle<()>>,
}

impl MeterEngine {
    pub fn start(device_ids: Vec<(String, bool)>) -> Result<Self> {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let peaks = Arc::new(Mutex::new(HashMap::new()));
        let mut threads = Vec::new();

        for (dev_id, is_loopback) in device_ids {
            let stop = stop_flag.clone();
            let peaks_ref = peaks.clone();
            let id = dev_id.clone();
            let handle = thread::Builder::new()
                .name(format!("meter-{}", &id[..8.min(id.len())]))
                .spawn(move || {
                    if let Err(e) = meter_thread(&stop, &peaks_ref, &id, is_loopback) {
                        eprintln!("[meter {}] error: {e:#}", &id[..8.min(id.len())]);
                    }
                })
                .context("spawn meter thread")?;
            threads.push(handle);
        }

        Ok(Self { stop_flag, peaks, threads })
    }

    pub fn get_peaks(&self) -> HashMap<String, f32> {
        self.peaks.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn stop(mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        for h in self.threads.drain(..) {
            let _ = h.join();
        }
    }
}

impl Drop for MeterEngine {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}

fn meter_thread(
    stop: &AtomicBool,
    peaks: &Mutex<HashMap<String, f32>>,
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
                AUDCLNT_STREAMFLAGS_LOOPBACK & !AUDCLNT_STREAMFLAGS_LOOPBACK
            };
            audio_client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 200_000, 0, mix_format, None)?;

            let capture_client: IAudioCaptureClient = audio_client.GetService()?;
            audio_client.Start()?;

            let mut peak: f32 = 0.0;
            let mut sample_count = 0u32;

            while !stop.load(Ordering::Relaxed) {
                thread::sleep(std::time::Duration::from_millis(15));

                let mut pkt = capture_client.GetNextPacketSize()?;
                while pkt > 0 {
                    let mut buf = std::ptr::null_mut();
                    let mut nframes = 0u32;
                    let mut flags_out = 0u32;
                    capture_client.GetBuffer(&mut buf, &mut nframes, &mut flags_out, None, None)?;

                    let frame_size = { (*mix_format).nBlockAlign } as usize;
                    let byte_count = nframes as usize * frame_size;
                    let data = std::slice::from_raw_parts(buf as *const u8, byte_count);

                    // Compute peak (interpret as f32 samples)
                    let samples = std::slice::from_raw_parts(
                        data.as_ptr() as *const f32,
                        byte_count / 4,
                    );
                    for &s in samples {
                        let abs = s.abs();
                        if abs > peak { peak = abs; }
                    }
                    sample_count += nframes;

                    capture_client.ReleaseBuffer(nframes)?;
                    pkt = capture_client.GetNextPacketSize()?;
                }

                // Update peak every ~30ms worth of samples
                if sample_count > 0 {
                    if let Ok(mut p) = peaks.lock() {
                        p.insert(device_id.to_string(), peak.min(1.0));
                    }
                    // Decay
                    peak *= 0.7;
                    sample_count = 0;
                }
            }

            audio_client.Stop()?;
            Ok(())
        })();

        if needs_uninit { CoUninitialize(); }
        result
    }
}
