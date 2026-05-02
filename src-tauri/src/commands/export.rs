use crate::audio_buffer::SAMPLE_RATE;
use crate::AppState;
use log::info;
use tauri_plugin_clipboard_manager::ClipboardExt;

#[tauri::command]
pub fn save_segment_as_wav(
    path: String,
    start: f32,
    end: f32,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    info!("[save_segment_as_wav] path={}, start={}, end={}", path, start, end);
    let audio = state.recorded_audio.blocking_write();
    if audio.len() == 0 {
        return Err("No recorded audio".to_string());
    }

    let start_sample = (start * SAMPLE_RATE as f32) as u64;
    let end_sample = (end * SAMPLE_RATE as f32) as u64;
    if start_sample >= end_sample {
        return Err("Invalid time range".to_string());
    }

    let segment = audio
        .snapshot_range(start_sample, end_sample)
        .ok_or("Requested segment is outside in-memory window")?;
    write_wav(&path, &segment)
}

#[tauri::command]
pub fn save_all_audio(path: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    info!("[save_all_audio] path={}", path);
    let audio = state.recorded_audio.blocking_write();
    if audio.len() == 0 {
        return Err("No recorded audio".to_string());
    }
    let samples = audio.snapshot_all();
    write_wav(&path, &samples)
}

#[tauri::command]
pub fn get_recorded_audio_path(state: tauri::State<'_, AppState>) -> Result<String, String> {
    info!("[get_recorded_audio_path]");
    let audio = state.recorded_audio.blocking_write();
    if audio.len() == 0 {
        return Err("No recorded audio".to_string());
    }

    let tmp = std::env::temp_dir().join(format!("sherpa-onnx-mic-{}.wav", std::process::id()));
    let tmp_str = tmp.to_str().ok_or("Invalid temp path")?.to_string();
    let samples = audio.snapshot_all();
    write_wav(&tmp_str, &samples)?;
    info!("[get_recorded_audio_path] wrote {tmp_str} ({} samples)", samples.len());
    Ok(tmp_str)
}

#[tauri::command]
pub fn export_srt(path: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    info!("[export_srt] path={}", path);
    let segments = state.segments.blocking_write();
    if segments.is_empty() {
        return Err("No results to export".to_string());
    }

    let mut srt = String::new();
    for (i, seg) in segments.iter().enumerate() {
        srt.push_str(&format!("{}\n", i + 1));
        srt.push_str(&format!(
            "{} --> {}\n",
            format_srt_time(seg.start),
            format_srt_time(seg.end)
        ));
        srt.push_str(&seg.text);
        srt.push_str("\n\n");
    }

    std::fs::write(&path, srt).map_err(|e| format!("Cannot write file: {e}"))?;
    Ok(())
}

#[tauri::command]
pub fn copy_text_to_clipboard(app: tauri::AppHandle, text: String) -> Result<(), String> {
    info!("[copy_text_to_clipboard] text_len={}", text.len());
    app.clipboard().write_text(text).map_err(|e| e.to_string())
}

fn format_srt_time(seconds: f32) -> String {
    let total_ms = (seconds * 1000.0) as u64;
    let h = total_ms / 3_600_000;
    let m = (total_ms % 3_600_000) / 60_000;
    let s = (total_ms % 60_000) / 1000;
    let ms = total_ms % 1000;
    format!("{h:02}:{m:02}:{s:02},{ms:03}")
}

/// Write mono f32 PCM samples as a 16-bit WAV file at 16 kHz.
fn write_wav(path: &str, samples: &[f32]) -> Result<(), String> {
    let num_samples = samples.len() as u32;
    let byte_rate = 16000u32 * 2;
    let data_size = num_samples * 2;
    let file_size = 36 + data_size;

    let f = std::fs::File::create(path).map_err(|e| format!("Cannot create file: {e}"))?;
    let mut w = std::io::BufWriter::new(f);

    use std::io::Write;
    w.write_all(b"RIFF").map_err(|e| e.to_string())?;
    w.write_all(&file_size.to_le_bytes()).map_err(|e| e.to_string())?;
    w.write_all(b"WAVE").map_err(|e| e.to_string())?;
    w.write_all(b"fmt ").map_err(|e| e.to_string())?;
    w.write_all(&16u32.to_le_bytes()).map_err(|e| e.to_string())?;
    w.write_all(&1u16.to_le_bytes()).map_err(|e| e.to_string())?;
    w.write_all(&1u16.to_le_bytes()).map_err(|e| e.to_string())?;
    w.write_all(&16000u32.to_le_bytes()).map_err(|e| e.to_string())?;
    w.write_all(&byte_rate.to_le_bytes()).map_err(|e| e.to_string())?;
    w.write_all(&2u16.to_le_bytes()).map_err(|e| e.to_string())?;
    w.write_all(&16u16.to_le_bytes()).map_err(|e| e.to_string())?;
    w.write_all(b"data").map_err(|e| e.to_string())?;
    w.write_all(&data_size.to_le_bytes()).map_err(|e| e.to_string())?;
    for &s in samples {
        let clamped = s.clamp(-1.0, 1.0);
        let pcm = (clamped * 32767.0) as i16;
        w.write_all(&pcm.to_le_bytes()).map_err(|e| e.to_string())?;
    }
    w.flush().map_err(|e| e.to_string())?;

    Ok(())
}
