use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use log::info;
use serde::Serialize;
use sherpa_onnx::LinearResampler;

use crate::audio_buffer::SAMPLE_RATE;
use crate::commands::recording::build_input_stream;
use crate::lock_utils::{mutex_lock, read_lock, write_lock};
use crate::AppState;

/// Seconds of audio captured for one enrollment.
const ENROLL_SECS: u64 = 6;

#[derive(Serialize)]
pub(crate) struct SpeakerStatus {
    /// A target voiceprint is enrolled.
    enrolled: bool,
    /// Gating is active.
    enabled: bool,
    /// Acceptance threshold (cosine similarity).
    threshold: f32,
    /// The embedding model loaded successfully.
    model_available: bool,
}

fn status_of(state: &AppState) -> SpeakerStatus {
    let sp = read_lock(&state.speaker);
    SpeakerStatus {
        enrolled: sp.is_enrolled(),
        enabled: sp.enabled,
        threshold: sp.threshold,
        model_available: sp.extractor.is_some(),
    }
}

async fn persist(state: &AppState) -> Result<(), String> {
    let (enabled, threshold, target) = {
        let sp = read_lock(&state.speaker);
        (sp.enabled, sp.threshold, sp.target.clone())
    };
    let db = {
        let g = mutex_lock(&state.db);
        g.as_ref().cloned().ok_or("Database not initialized")?
    };
    crate::settings::save_speaker_config_to_db(&db, enabled, threshold, target.as_ref()).await
}

#[tauri::command]
pub async fn get_speaker_status(state: tauri::State<'_, AppState>) -> Result<SpeakerStatus, String> {
    Ok(status_of(&state))
}

#[tauri::command]
pub async fn enroll_speaker(state: tauri::State<'_, AppState>) -> Result<SpeakerStatus, String> {
    info!("[enroll_speaker]");
    if state.recording.load(Ordering::SeqCst) {
        return Err("录音进行中，无法注册声纹，请先停止录音".to_string());
    }
    if read_lock(&state.speaker).extractor.is_none() {
        return Err("声纹模型未加载（缺少 speaker-embedding.onnx）".to_string());
    }

    let selected = read_lock(&state.selected_device).clone();

    // Capture ~ENROLL_SECS of mono audio, resampled to 16 kHz, on a blocking thread.
    let samples_16k = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<f32>, String> {
        let host = cpal::default_host();
        let device = if let Some(name) = selected {
            host.input_devices()
                .map_err(|e| format!("Cannot enumerate devices: {e}"))?
                .find(|d| d.name().ok().as_deref() == Some(name.as_str()))
                .ok_or_else(|| format!("Device not found: {name}"))?
        } else {
            host.default_input_device().ok_or("No default input device")?
        };
        let supported = device
            .default_input_config()
            .map_err(|e| format!("No input config: {e}"))?;
        let mic_rate = supported.sample_rate().0 as i32;
        let resampler = if mic_rate != SAMPLE_RATE as i32 {
            Some(
                LinearResampler::create(mic_rate, SAMPLE_RATE as i32)
                    .ok_or_else(|| format!("Failed to create resampler for {mic_rate} Hz"))?,
            )
        } else {
            None
        };

        let (tx, rx) = mpsc::channel::<Vec<f32>>();
        let received = Arc::new(AtomicBool::new(false));
        let stream = build_input_stream(&device, tx, Arc::clone(&received))?;
        stream.play().map_err(|e| format!("Stream play: {e}"))?;

        let mut out: Vec<f32> = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(ENROLL_SECS);
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(chunk) => {
                    let pcm = match resampler {
                        Some(ref r) => r.resample(&chunk, false),
                        None => chunk,
                    };
                    out.extend_from_slice(&pcm);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        drop(stream);

        if !received.load(Ordering::Relaxed) || out.len() < SAMPLE_RATE {
            return Err("未采集到足够的语音，请对着麦克风清晰说话约 6 秒后重试".to_string());
        }
        Ok(out)
    })
    .await
    .map_err(|e| format!("Enrollment task failed: {e}"))??;

    // Compute embedding and store as the target.
    {
        let mut sp = write_lock(&state.speaker);
        let ext = sp
            .extractor
            .as_ref()
            .ok_or("声纹模型未加载")?;
        let emb = crate::speaker::embed(ext, &samples_16k)
            .ok_or("声纹特征提取失败，请重试")?;
        sp.target = Some(emb);
        sp.enabled = true; // auto-enable after enrollment
    }
    persist(&state).await?;
    info!("[enroll_speaker] enrolled & gating enabled");
    Ok(status_of(&state))
}

#[tauri::command]
pub async fn clear_speaker(state: tauri::State<'_, AppState>) -> Result<SpeakerStatus, String> {
    info!("[clear_speaker]");
    {
        let mut sp = write_lock(&state.speaker);
        sp.target = None;
        sp.enabled = false;
    }
    persist(&state).await?;
    Ok(status_of(&state))
}

#[tauri::command]
pub async fn set_speaker_enabled(
    enabled: bool,
    state: tauri::State<'_, AppState>,
) -> Result<SpeakerStatus, String> {
    info!("[set_speaker_enabled] {enabled}");
    {
        let mut sp = write_lock(&state.speaker);
        if enabled && !sp.is_enrolled() {
            return Err("尚未注册声纹，无法开启音色门控".to_string());
        }
        sp.enabled = enabled;
    }
    persist(&state).await?;
    Ok(status_of(&state))
}

#[tauri::command]
pub async fn set_speaker_threshold(
    threshold: f32,
    state: tauri::State<'_, AppState>,
) -> Result<SpeakerStatus, String> {
    info!("[set_speaker_threshold] {threshold}");
    if !(0.0..=1.0).contains(&threshold) {
        return Err("阈值需在 0.0 ~ 1.0 之间".to_string());
    }
    {
        let mut sp = write_lock(&state.speaker);
        sp.threshold = threshold;
    }
    persist(&state).await?;
    Ok(status_of(&state))
}
