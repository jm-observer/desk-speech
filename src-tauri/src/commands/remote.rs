//! Remote ASR session (redesign / P0).
//!
//! Recording streams mic PCM to the GB10 orchestrator over WebSocket (see
//! docs/protocol-draft.md). The orchestrator URL is held in `AppState.remote_url`
//! and edited from the desktop UI (persisted as `remote.url` in SQLite).
//! Incoming protocol events are mapped to the existing `segment_updated`
//! frontend event so the UI is unchanged.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::Arc;
use std::time::Duration;

use chrono::{Local, NaiveDateTime};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use futures_util::{SinkExt, StreamExt};
use log::{error, info, warn};
use tauri::Emitter;
use tokio::sync::mpsc as tok_mpsc;
use tokio_tungstenite::tungstenite::Message;

use std::sync::RwLock;

use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::commands::notify::bounce_tray_twice;
use crate::commands::recording::build_input_stream;

/// Target sample rate for the upstream PCM the orchestrator expects.
const SAMPLE_RATE: u32 = 16_000;

use crate::llm_settings::{AutoCopyMode, LlmSettings};
use crate::lock_utils::read_lock;
use crate::settings::VadSettings;

/// Tracks the previous auto-copied segment on the **audio timeline** (not
/// the client's wall clock) so the merge decision is independent of LLM
/// round-trip latency. `t_end` is the end second of the last segment whose
/// text we wrote to the clipboard; the next segment merges if its
/// `t_start` is within `window` seconds of that value.
struct AutoCopyAccum {
    t_end: f64,
    text: String,
    ref_id: i64,
}

/// Decide what to actually paste into the clipboard for this segment.
///
/// Merges with the previous auto-copy when:
/// - the gap **on the audio timeline** (`t_start - prev.t_end`) is below
///   `window` — i.e. the user actually spoke them close together;
/// - the segment id differs (not a re-emit of the same segment);
/// - prev text is non-empty.
///
/// Using the audio timeline (not `Instant::elapsed`) means slow vLLM
/// optimization no longer breaks merging: two sentences spoken 1 s apart
/// stitch together even if each took 10 s to optimize. A zero `window`
/// disables merging entirely. Mutates `acc` in place to remember the
/// pasted text and `t_end` for the next call.
fn next_clipboard_text(
    acc: &mut Option<AutoCopyAccum>,
    text: &str,
    ref_id: i64,
    t_start: f64,
    t_end: f64,
    window: Duration,
) -> String {
    let window_secs = window.as_secs_f64();
    let merged = match acc.as_ref() {
        Some(prev)
            if (t_start - prev.t_end) < window_secs
                && prev.ref_id != ref_id
                && !prev.text.is_empty() =>
        {
            format!("{} {}", prev.text, text)
        }
        _ => text.to_string(),
    };
    *acc = Some(AutoCopyAccum {
        t_end,
        text: merged.clone(),
        ref_id,
    });
    merged
}

/// Add `secs` seconds to a `"YYYY-MM-DD HH:MM:SS"` wall-clock string,
/// returning the formatted result. Used to derive `wall_end` from
/// `wall_start` + segment duration so SegmentCard shows a real time range
/// instead of `15:42:46 → 15:42:46`. Falls back to the input on parse
/// failure or non-positive duration.
fn add_seconds_to_wall(wall: &str, secs: f64) -> String {
    if !(secs > 0.0) {
        return wall.to_string();
    }
    let Ok(dt) = NaiveDateTime::parse_from_str(wall, "%Y-%m-%d %H:%M:%S") else {
        return wall.to_string();
    };
    let added = dt + chrono::Duration::seconds(secs.round() as i64);
    added.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Returns the configured remote orchestrator URL from app state, if non-empty.
pub(crate) fn remote_url(state: &crate::AppState) -> Option<String> {
    let v = read_lock(&state.remote_url).clone();
    if v.trim().is_empty() { None } else { Some(v) }
}

/// Derive the orchestrator HTTP base (e.g. "http://192.168.0.68:8090")
/// from the active WebSocket URL (e.g. "ws://192.168.0.68:8090/stream").
fn remote_http_base(state: &crate::AppState) -> Option<String> {
    let ws = remote_url(state)?;
    let (scheme, rest) = if let Some(r) = ws.strip_prefix("wss://") {
        ("https://", r)
    } else if let Some(r) = ws.strip_prefix("ws://") {
        ("http://", r)
    } else {
        return None;
    };
    // strip everything from the first '/' onward (path), keep host[:port]
    let host = rest.split_once('/').map(|(h, _)| h).unwrap_or(rest);
    Some(format!("{scheme}{host}"))
}

/// Fetch recent transcribed segments from the orchestrator's `/api/history`.
/// Used by the desktop client to pre-populate the result list on startup
/// (last N transcripts) so the panel isn't empty before the user records.
#[tauri::command]
pub async fn fetch_remote_history(
    limit: u32,
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    let Some(base) = remote_http_base(&state) else {
        return Err("远程识别地址未配置".to_string());
    };
    let lim = limit.clamp(1, 200);
    let url = format!("{base}/api/history?limit={lim}");
    let resp = reqwest::get(&url).await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("history api status {}", resp.status()));
    }
    let body: Vec<serde_json::Value> = resp.json().await.map_err(|e| e.to_string())?;
    Ok(body)
}

/// Minimal stateful linear resampler (mono), replaces sherpa's resampler so
/// the client no longer depends on sherpa-onnx. ASR-grade quality is fine.
struct LinResampler {
    step: f64, // input samples advanced per output sample
    pos: f64,
    last: f32,
    have_last: bool,
}

impl LinResampler {
    fn new(in_rate: f64, out_rate: f64) -> Self {
        Self { step: in_rate / out_rate, pos: 0.0, last: 0.0, have_last: false }
    }

    fn process(&mut self, input: &[f32]) -> Vec<f32> {
        if input.is_empty() {
            return Vec::new();
        }
        let mut buf: Vec<f32> = Vec::with_capacity(input.len() + 1);
        if self.have_last {
            buf.push(self.last);
        }
        buf.extend_from_slice(input);
        let mut out = Vec::with_capacity(((buf.len() as f64) / self.step) as usize + 1);
        while (self.pos as usize) + 1 < buf.len() {
            let i = self.pos as usize;
            let frac = self.pos - i as f64;
            let s = buf[i] as f64 * (1.0 - frac) + buf[i + 1] as f64 * frac;
            out.push(s as f32);
            self.pos += self.step;
        }
        self.last = *buf.last().unwrap();
        self.have_last = true;
        self.pos -= (buf.len() - 1) as f64;
        if self.pos < 0.0 {
            self.pos = 0.0;
        }
        out
    }
}

fn now_rfc3339() -> String {
    // Full local timestamp ("YYYY-MM-DD HH:MM:SS") — matches the format the
    // server's /api/history returns (`ts`), so the desktop UI's sort
    // (string-compare on wall_start) keeps live segments and preloaded
    // history in chronological order. The UI's stripYear() trims the date
    // back off for display.
    Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Accumulated state of one segment (so updates never clobber prior fields).
#[derive(Default, Clone)]
struct SegState {
    raw: String,
    opt: Option<String>,
    eng: Option<String>,
    /// Secondary-recognizer transcription (dual-model comparison mode); only
    /// populated when the client opted in via `hello.want_secondary` and the
    /// orchestrator sends a paired `secondary` event.
    sec: Option<String>,
    sec_kind: Option<String>,
    t0: f64,
    t1: f64,
    wall: String,
    speaker: Option<String>,
    /// True after we've already flashed the taskbar for this segment, so a
    /// repeated event (or out-of-order updates) doesn't flash twice.
    flashed: bool,
}

/// Emit the *full current* segment state as `segment_updated` (DbSegmentDto
/// shape). Frontend merges by id; we always send the complete state so
/// optimized/translated never blank the raw text.
fn emit_state(app: &tauri::AppHandle, id: i64, s: &SegState) {
    let optimize_status = if s.opt.is_some() { "success" } else { "running" };
    let translate_status = if s.eng.is_some() { "success" } else { "running" };
    info!(
        "[remote][emit] id={id} raw={:?} opt={:?} eng={:?} sec={:?} t=[{:.2},{:.2}]",
        s.raw, s.opt, s.eng, s.sec, s.t0, s.t1
    );
    // wall_end = wall_start + (t1 - t0). Without this both fields shared
    // the single first-event timestamp and SegmentCard showed
    // `15:42:46 → 15:42:46` regardless of segment length.
    let wall_end = add_seconds_to_wall(&s.wall, s.t1 - s.t0);
    let _ = app.emit(
        "segment_updated",
        serde_json::json!({
            "id": id,
            "segment_id": id,
            "revision": id,
            "start_sec": s.t0,
            "end_sec": s.t1,
            "wall_start": s.wall,
            "wall_end": wall_end,
            "text_raw": s.raw,
            "optimize_status": optimize_status,
            "translate_status": translate_status,
            "text_optimized": s.opt,
            "text_english": s.eng,
            "text_secondary": s.sec,
            "secondary_kind": s.sec_kind,
            "speaker": s.speaker,
            "created_at": s.wall,
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
        let mut resampler = if mic_rate != SAMPLE_RATE as i32 {
            Some(LinResampler::new(mic_rate as f64, SAMPLE_RATE as f64))
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
                        Some(ref mut r) => r.process(&frame),
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
#[derive(PartialEq)]
enum Outcome {
    Stopped,      // user stopped recording -> end session
    Disconnected, // socket dropped while still recording -> try to reconnect
}

/// Max consecutive connect failures before giving up and showing an error.
const MAX_CONN_FAILS: u32 = 4;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_remote_session(
    url: String,
    app: tauri::AppHandle,
    selected_device: Arc<RwLock<Option<String>>>,
    settings: Arc<RwLock<VadSettings>>,
    llm_settings: Arc<RwLock<LlmSettings>>,
    stop_signal: Arc<AtomicBool>,
    recording: Arc<AtomicBool>,
    init_status: Arc<AtomicU8>,
    init_error: Arc<RwLock<String>>,
) {
    let device_name = read_lock(&selected_device).clone();
    let language = {
        let s = read_lock(&settings);
        if s.asr_language.is_empty() { "auto".to_string() } else { s.asr_language.clone() }
    };
    // Dual-model comparison opt-in: read once at session start. Toggling
    // mid-session requires a stop/start (matches the URL-change reconnect).
    let want_secondary = read_lock(&llm_settings).want_secondary;
    let stop = stop_signal;

    let mut pcm_rx = match spawn_capture(device_name, Arc::clone(&stop)) {
        Ok(rx) => rx,
        Err(e) => {
            error!("[remote] capture init failed: {e}");
            *init_error.write().unwrap() = format!("麦克风初始化失败: {e}");
            init_status.store(2, Ordering::Relaxed);
            recording.store(false, Ordering::SeqCst);
            return;
        }
    };

    let hello = serde_json::json!({
        "type": "hello", "protocol": "1", "sample_rate": 16000,
        "format": "pcm_s16le", "language": language,
        "want_optimize": true, "want_translate": true,
        "want_secondary": want_secondary,
    })
    .to_string();

    let mut fails: u32 = 0;
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        match tokio_tungstenite::connect_async(&url).await {
            Ok((ws, _)) => {
                fails = 0;
                info!("[remote] connected {url}");
                let outcome = run_one_connection(
                    ws, &hello, &mut pcm_rx, &app, &llm_settings, &stop,
                )
                .await;
                if outcome == Outcome::Stopped || stop.load(Ordering::Relaxed) {
                    break;
                }
                warn!("[remote] disconnected mid-session; reconnecting...");
            }
            Err(e) => {
                fails += 1;
                error!("[remote] connect {url} failed ({fails}/{MAX_CONN_FAILS}): {e}");
                if fails >= MAX_CONN_FAILS {
                    *init_error.write().unwrap() =
                        format!("无法连接识别服务 {url}: {e}");
                    init_status.store(2, Ordering::Relaxed);
                    break;
                }
                let backoff = Duration::from_secs(1u64 << fails.min(3));
                tokio::time::sleep(backoff).await;
            }
        }
    }

    stop.store(true, Ordering::Relaxed); // stop capture thread
    recording.store(false, Ordering::SeqCst);
    info!("[remote] session ended");
}

/// One WebSocket connection: hello -> stream PCM, forward events. Returns
/// whether the user stopped (end) or the socket dropped (reconnect).
async fn run_one_connection(
    ws: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    hello: &str,
    pcm_rx: &mut tok_mpsc::UnboundedReceiver<Vec<u8>>,
    app: &tauri::AppHandle,
    llm_settings: &Arc<RwLock<LlmSettings>>,
    stop: &Arc<AtomicBool>,
) -> Outcome {
    let (mut wr, mut rd) = ws.split();
    if wr.send(Message::Text(hello.to_string())).await.is_err() {
        return Outcome::Disconnected;
    }

    let app_r = app.clone();
    let llm_settings_r = Arc::clone(llm_settings);
    let mut reader = tokio::spawn(async move {
        let mut segs: HashMap<i64, SegState> = HashMap::new();
        // Stitch window accumulator: appends short-gap copies into one paste.
        let mut copy_acc: Option<AutoCopyAccum> = None;
        while let Some(Ok(msg)) = rd.next().await {
            let Message::Text(t) = msg else { continue };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) else { continue };
            match v.get("type").and_then(|x| x.as_str()) {
                Some("ready") => info!("[remote] session ready"),
                Some("segment") => {
                    let id = v.get("id").and_then(|x| x.as_i64()).unwrap_or(0);
                    let text = v.get("text").and_then(|x| x.as_str()).unwrap_or("");
                    let t0 = v.get("t_start").and_then(|x| x.as_f64());
                    let t1 = v.get("t_end").and_then(|x| x.as_f64());
                    info!("[remote][segment] id={id} t=[{t0:?},{t1:?}] text={text:?}");
                    let st = segs.entry(id).or_default();
                    st.raw = text.to_string();
                    st.t0 = v.get("t_start").and_then(|x| x.as_f64()).unwrap_or(st.t0);
                    st.t1 = v.get("t_end").and_then(|x| x.as_f64()).unwrap_or(st.t1);
                    if let Some(sp) = v.get("speaker").and_then(|x| x.as_str()) {
                        st.speaker = Some(sp.to_string());
                    }
                    if st.wall.is_empty() {
                        st.wall = now_rfc3339();
                    }
                    emit_state(&app_r, id, st);
                }
                Some("optimized") => {
                    let id = v.get("ref").and_then(|x| x.as_i64()).unwrap_or(0);
                    let text = v.get("text").and_then(|x| x.as_str()).unwrap_or("").to_string();
                    info!("[remote][optimized] ref={id} text={text:?}");
                    let st = segs.entry(id).or_default();
                    if st.wall.is_empty() {
                        st.wall = now_rfc3339();
                    }
                    st.opt = Some(text.clone());
                    emit_state(&app_r, id, st);
                    info!(
                        "[remote][flash-check] after optimized id={id} opt={} eng={} flashed={}",
                        st.opt.is_some(), st.eng.is_some(), st.flashed
                    );
                    if !st.flashed && st.opt.is_some() && st.eng.is_some() {
                        info!("[remote][flash-trigger] id={id} (triggered by optimized)");
                        st.flashed = true;
                        let play_beep = read_lock(&llm_settings_r).notify_sound;
                        bounce_tray_twice(&app_r, play_beep);
                    }
                    let (copy, window_ms) = {
                        let s = read_lock(&llm_settings_r);
                        (matches!(s.auto_copy_mode, AutoCopyMode::OptimizedZh), s.merge_window_ms)
                    };
                    if copy && !text.is_empty() {
                        let merged = next_clipboard_text(
                            &mut copy_acc,
                            &text,
                            id,
                            st.t0,
                            st.t1,
                            Duration::from_millis(window_ms),
                        );
                        let merged_for_log = merged.clone();
                        match app_r.clipboard().write_text(merged) {
                            Ok(_) => info!(
                                "[remote] auto copy (优化中文) ref={id} chars={}",
                                merged_for_log.chars().count()
                            ),
                            Err(e) => error!("[remote] clipboard 优化中文 failed: {e}"),
                        }
                    }
                }
                Some("translated") => {
                    let id = v.get("ref").and_then(|x| x.as_i64()).unwrap_or(0);
                    let text = v.get("text").and_then(|x| x.as_str()).unwrap_or("").to_string();
                    info!("[remote][translated] ref={id} text={text:?}");
                    let st = segs.entry(id).or_default();
                    if st.wall.is_empty() {
                        st.wall = now_rfc3339();
                    }
                    st.eng = Some(text.clone());
                    emit_state(&app_r, id, st);
                    info!(
                        "[remote][flash-check] after translated id={id} opt={} eng={} flashed={}",
                        st.opt.is_some(), st.eng.is_some(), st.flashed
                    );
                    if !st.flashed && st.opt.is_some() && st.eng.is_some() {
                        info!("[remote][flash-trigger] id={id} (triggered by translated)");
                        st.flashed = true;
                        let play_beep = read_lock(&llm_settings_r).notify_sound;
                        bounce_tray_twice(&app_r, play_beep);
                    }
                    let (copy, window_ms) = {
                        let s = read_lock(&llm_settings_r);
                        (matches!(s.auto_copy_mode, AutoCopyMode::English), s.merge_window_ms)
                    };
                    if copy && !text.is_empty() {
                        let merged = next_clipboard_text(
                            &mut copy_acc,
                            &text,
                            id,
                            st.t0,
                            st.t1,
                            Duration::from_millis(window_ms),
                        );
                        let merged_for_log = merged.clone();
                        match app_r.clipboard().write_text(merged) {
                            Ok(_) => info!(
                                "[remote] auto copy (英文) ref={id} chars={}",
                                merged_for_log.chars().count()
                            ),
                            Err(e) => error!("[remote] clipboard 英文 failed: {e}"),
                        }
                    }
                }
                Some("secondary") => {
                    let id = v.get("ref").and_then(|x| x.as_i64()).unwrap_or(0);
                    let text = v.get("text").and_then(|x| x.as_str()).unwrap_or("").to_string();
                    let kind = v.get("kind").and_then(|x| x.as_str()).map(str::to_string);
                    info!("[remote][secondary] ref={id} kind={:?} text={text:?}", kind);
                    let st = segs.entry(id).or_default();
                    if st.wall.is_empty() {
                        st.wall = now_rfc3339();
                    }
                    st.sec = Some(text);
                    st.sec_kind = kind;
                    emit_state(&app_r, id, st);
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

    // Writer: pump PCM. Returns Stopped if the user stopped, Disconnected if
    // the socket dropped (so the caller can reconnect).
    loop {
        if stop.load(Ordering::Relaxed) {
            let _ = wr
                .send(Message::Text(r#"{"type":"stop"}"#.to_string()))
                .await;
            let _ = tokio::time::timeout(Duration::from_secs(20), &mut reader).await;
            return Outcome::Stopped;
        }
        if reader.is_finished() {
            // server closed / socket dropped while still recording
            return Outcome::Disconnected;
        }
        match tokio::time::timeout(Duration::from_millis(200), pcm_rx.recv()).await {
            Ok(Some(bytes)) => {
                if wr.send(Message::Binary(bytes)).await.is_err() {
                    reader.abort();
                    return Outcome::Disconnected;
                }
            }
            Ok(None) => {
                reader.abort();
                return Outcome::Disconnected;
            }
            Err(_) => continue, // timeout: re-check stop / reader
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    // ---- next_clipboard_text: audio-timeline merge semantics ----

    #[test]
    fn first_call_writes_text_as_is() {
        let mut acc = None;
        let out = next_clipboard_text(&mut acc, "你好", 1, 0.0, 2.0, w(3000));
        assert_eq!(out, "你好");
        let a = acc.as_ref().unwrap();
        assert_eq!(a.text, "你好");
        assert_eq!(a.t_end, 2.0);
        assert_eq!(a.ref_id, 1);
    }

    #[test]
    fn merges_when_audio_gap_within_window() {
        let mut acc = None;
        next_clipboard_text(&mut acc, "你好", 1, 0.0, 2.0, w(3000));
        // seg2 starts at 4.0 s, prev ended at 2.0 s → audio gap 2 s, < 3 s
        let out = next_clipboard_text(&mut acc, "世界", 2, 4.0, 6.0, w(3000));
        assert_eq!(out, "你好 世界");
        assert_eq!(acc.as_ref().unwrap().t_end, 6.0);
    }

    #[test]
    fn does_not_merge_when_audio_gap_exceeds_window() {
        let mut acc = None;
        next_clipboard_text(&mut acc, "A", 1, 0.0, 2.0, w(3000));
        // seg2 starts at 10.0 s, gap = 8 s, > 3 s
        let out = next_clipboard_text(&mut acc, "B", 2, 10.0, 11.0, w(3000));
        assert_eq!(out, "B");
    }

    #[test]
    fn merge_ignores_real_time_only_audio_timeline() {
        // Simulates slow LLM: real-time gap between two clipboard writes
        // could be huge, but the audio gap is tiny → must merge. This is
        // the whole point of switching off Instant::elapsed.
        let mut acc = None;
        next_clipboard_text(&mut acc, "上半句", 1, 10.0, 12.0, w(3000));
        // Pretend many real seconds passed (LLM was slow), but on the
        // audio timeline seg2 follows immediately.
        std::thread::sleep(std::time::Duration::from_millis(50));
        let out = next_clipboard_text(&mut acc, "下半句", 2, 12.5, 14.0, w(3000));
        assert_eq!(out, "上半句 下半句");
    }

    #[test]
    fn does_not_merge_same_ref_id() {
        // A re-emit of the same segment must not concatenate with itself.
        let mut acc = None;
        next_clipboard_text(&mut acc, "A", 1, 0.0, 2.0, w(3000));
        let out = next_clipboard_text(&mut acc, "A v2", 1, 2.5, 4.0, w(3000));
        assert_eq!(out, "A v2");
    }

    #[test]
    fn zero_window_disables_merging() {
        let mut acc = None;
        next_clipboard_text(&mut acc, "A", 1, 0.0, 2.0, w(0));
        let out = next_clipboard_text(&mut acc, "B", 2, 2.0, 3.0, w(0));
        assert_eq!(out, "B");
    }

    #[test]
    fn chain_grows_across_many_segments() {
        let mut acc = None;
        next_clipboard_text(&mut acc, "一", 1, 0.0, 1.0, w(3000));
        next_clipboard_text(&mut acc, "二", 2, 1.5, 2.5, w(3000));
        let out = next_clipboard_text(&mut acc, "三", 3, 3.0, 4.0, w(3000));
        assert_eq!(out, "一 二 三");
    }

    #[test]
    fn chain_resets_after_long_pause() {
        let mut acc = None;
        next_clipboard_text(&mut acc, "一", 1, 0.0, 1.0, w(3000));
        next_clipboard_text(&mut acc, "二", 2, 1.5, 2.5, w(3000));
        // 10 s of silence on the audio timeline → fresh chain
        let out = next_clipboard_text(&mut acc, "三", 3, 12.5, 13.5, w(3000));
        assert_eq!(out, "三");
    }

    // ---- add_seconds_to_wall: wall_end derivation ----

    #[test]
    fn wall_end_adds_rounded_duration() {
        let out = add_seconds_to_wall("2026-05-27 15:42:46", 9.4);
        assert_eq!(out, "2026-05-27 15:42:55");
    }

    #[test]
    fn wall_end_rounds_half_up() {
        let out = add_seconds_to_wall("2026-05-27 15:42:46", 0.6);
        assert_eq!(out, "2026-05-27 15:42:47");
    }

    #[test]
    fn wall_end_zero_or_negative_returns_input() {
        assert_eq!(
            add_seconds_to_wall("2026-05-27 15:42:46", 0.0),
            "2026-05-27 15:42:46"
        );
        assert_eq!(
            add_seconds_to_wall("2026-05-27 15:42:46", -3.0),
            "2026-05-27 15:42:46"
        );
    }

    #[test]
    fn wall_end_falls_back_on_parse_failure() {
        assert_eq!(add_seconds_to_wall("not a date", 5.0), "not a date");
    }

    #[test]
    fn wall_end_crosses_minute_boundary() {
        let out = add_seconds_to_wall("2026-05-27 15:42:58", 5.0);
        assert_eq!(out, "2026-05-27 15:43:03");
    }
}
