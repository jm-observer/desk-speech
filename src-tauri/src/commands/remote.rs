//! Remote ASR session (redesign / P0).
//!
//! When `REMOTE_ASR_URL` is set, recording streams mic PCM to the GB10
//! orchestrator over WebSocket (see docs/protocol-draft.md) instead of running
//! sherpa-onnx locally. Incoming protocol events are mapped to the existing
//! `segment_updated` frontend event so the UI is unchanged.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::Arc;
use std::time::Duration;

use chrono::Local;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use futures_util::{SinkExt, StreamExt};
use log::{error, info, warn};
use sherpa_onnx::LinearResampler;
use tauri::Emitter;
use tokio::sync::mpsc as tok_mpsc;
use tokio_tungstenite::tungstenite::Message;

use std::sync::RwLock;

use crate::audio_buffer::SAMPLE_RATE;
use crate::commands::recording::build_input_stream;
use crate::lock_utils::read_lock;
use crate::settings::VadSettings;

/// Returns the configured remote orchestrator URL, if any.
pub(crate) fn remote_url() -> Option<String> {
    std::env::var("REMOTE_ASR_URL").ok().filter(|s| !s.is_empty())
}

fn now_rfc3339() -> String {
    Local::now().to_rfc3339()
}

/// Emit a `segment_updated` event shaped like `DbSegmentDto` (frontend unchanged).
#[allow(clippy::too_many_arguments)]
fn emit_segment(
    app: &tauri::AppHandle,
    id: i64,
    text_raw: &str,
    text_optimized: Option<&str>,
    text_english: Option<&str>,
    optimize_status: &str,
    translate_status: &str,
) {
    let ts = now_rfc3339();
    let _ = app.emit(
        "segment_updated",
        serde_json::json!({
            "id": id,
            "segment_id": id,
            "revision": id,
            "start_sec": 0.0,
            "end_sec": 0.0,
            "wall_start": ts,
            "wall_end": ts,
            "text_raw": text_raw,
            "optimize_status": optimize_status,
            "translate_status": translate_status,
            "text_optimized": text_optimized,
            "text_english": text_english,
            "created_at": ts,
        }),
    );
}

/// Spawn the capture thread; returns a receiver of 16 kHz mono s16le PCM chunks.
fn spawn_capture(
    device_name: Option<String>,
    stop: Arc<AtomicBool>,
) -> Result<tok_mpsc::UnboundedReceiver<Vec<u8>>, String> {
    let (pcm_tx, pcm_rx) = tok_mpsc::unbounded_channel::<Vec<u8>>();

    std::thread::spawn(move || {
        let host = cpal::default_host();
        let device = match device_name {
            Some(name) => host
                .input_devices()
                .ok()
                .and_then(|mut it| it.find(|d| d.name().ok().as_deref() == Some(name.as_str()))),
            None => host.default_input_device(),
        };
        let Some(device) = device else {
            error!("[remote] no input device");
            return;
        };
        let Ok(supported) = device.default_input_config() else {
            error!("[remote] no input config");
            return;
        };
        let mic_rate = supported.sample_rate().0 as i32;
        let resampler = if mic_rate != SAMPLE_RATE as i32 {
            LinearResampler::create(mic_rate, SAMPLE_RATE as i32)
        } else {
            None
        };

        let (tx, rx) = std_mpsc::channel::<Vec<f32>>();
        let received = Arc::new(AtomicBool::new(false));
        let stream = match build_input_stream(&device, tx, Arc::clone(&received)) {
            Ok(s) => s,
            Err(e) => {
                error!("[remote] build stream: {e}");
                return;
            }
        };
        if let Err(e) = stream.play() {
            error!("[remote] stream play: {e}");
            return;
        }
        info!("[remote] capture started (mic {mic_rate} Hz -> {SAMPLE_RATE})");

        while !stop.load(Ordering::Relaxed) {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(frame) => {
                    let pcm16k: Vec<f32> = match resampler {
                        Some(ref r) => r.resample(&frame, false),
                        None => frame,
                    };
                    let mut bytes = Vec::with_capacity(pcm16k.len() * 2);
                    for s in pcm16k {
                        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
                        bytes.extend_from_slice(&v.to_le_bytes());
                    }
                    if pcm_tx.send(bytes).is_err() {
                        break;
                    }
                }
                Err(std_mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std_mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        drop(stream);
        info!("[remote] capture stopped");
    });

    Ok(pcm_rx)
}

/// Run one remote recording session. Returns when stopped or the socket closes.
pub(crate) async fn run_remote_session(
    url: String,
    app: tauri::AppHandle,
    selected_device: Arc<RwLock<Option<String>>>,
    settings: Arc<RwLock<VadSettings>>,
    stop_signal: Arc<AtomicBool>,
    recording: Arc<AtomicBool>,
) {
    let device_name = read_lock(&selected_device).clone();
    let language = {
        let s = read_lock(&settings);
        if s.asr_language.is_empty() { "auto".to_string() } else { s.asr_language.clone() }
    };
    let stop = stop_signal;

    let mut pcm_rx = match spawn_capture(device_name, Arc::clone(&stop)) {
        Ok(rx) => rx,
        Err(e) => {
            error!("[remote] capture init failed: {e}");
            recording.store(false, Ordering::SeqCst);
            return;
        }
    };

    let conn = tokio_tungstenite::connect_async(&url).await;
    let ws = match conn {
        Ok((ws, _)) => ws,
        Err(e) => {
            error!("[remote] connect {url} failed: {e}");
            stop.store(true, Ordering::Relaxed);
            recording.store(false, Ordering::SeqCst);
            return;
        }
    };
    info!("[remote] connected {url}");
    let (mut wr, mut rd) = ws.split();

    let hello = serde_json::json!({
        "type": "hello", "protocol": "1", "sample_rate": 16000,
        "format": "pcm_s16le", "language": language,
        "want_optimize": true, "want_translate": true,
    })
    .to_string();
    if wr.send(Message::Text(hello)).await.is_err() {
        error!("[remote] failed to send hello");
        stop.store(true, Ordering::Relaxed);
        recording.store(false, Ordering::SeqCst);
        return;
    }

    // Reader: map protocol events -> segment_updated
    let app_r = app.clone();
    let reader = tokio::spawn(async move {
        while let Some(Ok(msg)) = rd.next().await {
            let Message::Text(t) = msg else { continue };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) else { continue };
            match v.get("type").and_then(|x| x.as_str()) {
                Some("ready") => info!("[remote] session ready"),
                Some("segment") => {
                    let id = v.get("id").and_then(|x| x.as_i64()).unwrap_or(0);
                    let text = v.get("text").and_then(|x| x.as_str()).unwrap_or("");
                    emit_segment(&app_r, id, text, None, None, "running", "running");
                }
                Some("optimized") => {
                    let id = v.get("ref").and_then(|x| x.as_i64()).unwrap_or(0);
                    let text = v.get("text").and_then(|x| x.as_str()).unwrap_or("");
                    emit_segment(&app_r, id, "", Some(text), None, "success", "running");
                }
                Some("translated") => {
                    let id = v.get("ref").and_then(|x| x.as_i64()).unwrap_or(0);
                    let text = v.get("text").and_then(|x| x.as_str()).unwrap_or("");
                    emit_segment(&app_r, id, "", None, Some(text), "success", "success");
                }
                Some("error") => {
                    warn!("[remote] server error: {}", v.get("message").and_then(|x| x.as_str()).unwrap_or(""));
                }
                Some("done") => {
                    info!("[remote] server done");
                    break;
                }
                _ => {}
            }
        }
    });

    // Writer: pump PCM until stop, then send stop and let reader drain.
    loop {
        if stop.load(Ordering::Relaxed) {
            let _ = wr.send(Message::Text(r#"{"type":"stop"}"#.to_string())).await;
            break;
        }
        match tokio::time::timeout(Duration::from_millis(200), pcm_rx.recv()).await {
            Ok(Some(bytes)) => {
                if wr.send(Message::Binary(bytes)).await.is_err() {
                    break;
                }
            }
            Ok(None) => break,
            Err(_) => continue, // timeout: re-check stop
        }
    }

    let _ = tokio::time::timeout(Duration::from_secs(20), reader).await;
    recording.store(false, Ordering::SeqCst);
    info!("[remote] session ended");
}
