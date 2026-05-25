//! Recording command surface (remote-only client).
//!
//! Recognition runs on the GB10 orchestrator (see commands/remote.rs); this
//! module only owns mic capture plumbing and the start/stop/clear/state
//! Tauri commands. The legacy local sherpa-onnx pipeline has been removed.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;

use cpal::traits::DeviceTrait;
use cpal::SampleFormat;
use log::info;

use crate::lock_utils::write_lock;
use crate::{AppState, RecordingState};

#[tauri::command]
pub async fn start_recording(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    info!("[start_recording]");
    if state.recording.swap(true, Ordering::SeqCst) {
        return Err("Already recording".to_string());
    }

    // Remote-only: stream mic to the GB10 orchestrator.
    let Some(url) = crate::commands::remote::remote_url(&state) else {
        state.recording.store(false, Ordering::SeqCst);
        return Err("远程识别地址未配置(请在控制面板里设置)".to_string());
    };
    info!("[start_recording] remote mode -> {url}");
    state.stop_signal.store(false, Ordering::Relaxed);
    // Clear any prior error state so the UI returns to ready on (re)start.
    state.init_status.store(1, Ordering::Relaxed);
    *write_lock(&state.init_error) = String::new();

    let app2 = app.clone();
    let selected_device = Arc::clone(&state.selected_device);
    let settings = Arc::clone(&state.settings);
    let llm_settings = Arc::clone(&state.llm_settings);
    let stop_signal = Arc::clone(&state.stop_signal);
    let recording = Arc::clone(&state.recording);
    let init_status = Arc::clone(&state.init_status);
    let init_error = Arc::clone(&state.init_error);
    tauri::async_runtime::spawn(async move {
        crate::commands::remote::run_remote_session(
            url, app2, selected_device, settings, llm_settings, stop_signal, recording,
            init_status, init_error,
        )
        .await;
    });
    Ok(())
}

/// Build a cpal input stream that pushes mono f32 frames (at the device's
/// native rate) into `tx`. Resampling to 16 kHz happens downstream.
pub(crate) fn build_input_stream(
    device: &cpal::Device,
    tx: mpsc::Sender<Vec<f32>>,
    received_audio: Arc<AtomicBool>,
) -> Result<cpal::Stream, String> {
    let supported = device
        .default_input_config()
        .map_err(|e| format!("No input config: {e}"))?;
    let config = supported.config();
    let sample_format = supported.sample_format();
    let channels = config.channels as usize;
    if channels == 0 {
        return Err("Device reports 0 channels".to_string());
    }

    info!(
        "[mic] format: {:?}, channels: {}, sample_rate: {}",
        sample_format, channels, config.sample_rate.0
    );

    let err_fn = |err| info!("[mic] stream error: {:?}", err);

    let stream = match sample_format {
        SampleFormat::F32 => device
            .build_input_stream(
                &config,
                move |data: &[f32], _| {
                    if data.is_empty() {
                        return;
                    }
                    if !received_audio.swap(true, Ordering::Relaxed) {
                        info!("[mic] first audio callback received, frames={}", data.len() / channels);
                    }
                    let mono: Vec<f32> = data
                        .chunks(channels)
                        .map(|frame| frame.iter().copied().sum::<f32>() / channels as f32)
                        .collect();
                    let _ = tx.send(mono);
                },
                err_fn,
                None,
            )
            .map_err(|e| format!("Build F32 stream: {e}"))?,

        SampleFormat::I16 => device
            .build_input_stream(
                &config,
                move |data: &[i16], _| {
                    if data.is_empty() {
                        return;
                    }
                    if !received_audio.swap(true, Ordering::Relaxed) {
                        info!("[mic] first audio callback received, frames={}", data.len() / channels);
                    }
                    let mono: Vec<f32> = data
                        .chunks(channels)
                        .map(|frame| {
                            frame.iter().map(|&s| s as f32 / i16::MAX as f32).sum::<f32>()
                                / channels as f32
                        })
                        .collect();
                    let _ = tx.send(mono);
                },
                err_fn,
                None,
            )
            .map_err(|e| format!("Build I16 stream: {e}"))?,

        SampleFormat::U16 => device
            .build_input_stream(
                &config,
                move |data: &[u16], _| {
                    if data.is_empty() {
                        return;
                    }
                    if !received_audio.swap(true, Ordering::Relaxed) {
                        info!("[mic] first audio callback received, frames={}", data.len() / channels);
                    }
                    let mono: Vec<f32> = data
                        .chunks(channels)
                        .map(|frame| {
                            frame.iter().map(|&s| (s as f32 - 32768.0) / 32768.0).sum::<f32>()
                                / channels as f32
                        })
                        .collect();
                    let _ = tx.send(mono);
                },
                err_fn,
                None,
            )
            .map_err(|e| format!("Build U16 stream: {e}"))?,

        other => return Err(format!("Unsupported sample format: {:?}", other)),
    };

    Ok(stream)
}

#[tauri::command]
pub fn stop_recording(state: tauri::State<'_, AppState>) {
    info!("[stop_recording] signalling stop");
    state.stop_signal.store(true, Ordering::Relaxed);
}

#[tauri::command]
pub fn clear_results(state: tauri::State<'_, AppState>) -> Result<(), String> {
    info!("[clear_results]");
    if state.recording.load(Ordering::SeqCst) {
        return Err("Cannot clear while recording".to_string());
    }
    Ok(())
}

#[tauri::command]
pub fn get_recording_state(state: tauri::State<'_, AppState>) -> Result<RecordingState, String> {
    let recording = state.recording.load(Ordering::Relaxed);
    Ok(RecordingState { recording })
}
