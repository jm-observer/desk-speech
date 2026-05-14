use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::Arc;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use chrono::Local;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use log::{debug, error, info, warn};
use sherpa_onnx::{LinearResampler, OfflineRecognizer, OfflineStream, VoiceActivityDetector};
use tauri::Emitter;
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::audio_buffer::SAMPLE_RATE;
use crate::db::repository::NewSegment;
use crate::db_worker::DbEvent;
use crate::llm_client::{
    check_discard_rules, evaluate_judgment, judge_discard, optimize_text, translate_text, JudgmentInput,
};
use crate::llm_settings::{AutoCopyMode, LlmSettings};
use crate::{
    can_start_finalization_check, merge_segment_in_place, mutex_lock, read_lock, set_segment_discard_state,
    set_segment_finalization_state, update_segment_llm_state, write_lock, AppState, RecognizeContext, RecordingAnchor,
    RecordingRuntime, RecordingState, SegmentResult, FINALIZE_SILENCE_MS,
};

#[tauri::command]
pub async fn start_recording(app: tauri::AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    info!("[start_recording]");
    info!("[start_recording] finalization_silence_ms={FINALIZE_SILENCE_MS}");
    if state.recording.swap(true, Ordering::SeqCst) {
        return Err("Already recording".to_string());
    }

    let init = state.init_status.load(Ordering::Relaxed);
    if init != 1 {
        state.recording.store(false, Ordering::SeqCst);
        return Err("Models not ready".to_string());
    }

    let recognizer = {
        let mut guard = write_lock(&state.recognizer);
        guard.take().ok_or("Recognizer not available")?
    };
    let vad = {
        let mut guard = write_lock(&state.vad);
        guard.take().ok_or("VAD not available")?
    };

    state.stop_signal.store(false, Ordering::Relaxed);

    // Reset session state
    {
        let mut segs = write_lock(&state.segments);
        segs.clear();
    }
    let audio_offset = read_lock(&state.recorded_audio).global_end_sample();

    let now = Local::now();
    *write_lock(&state.start_wall_clock) = Some(now);
    *write_lock(&state.start_instant) = Some(Instant::now());

    info!("[start_recording] starting at {now}");

    let db = {
        let guard = mutex_lock(&state.db);
        guard.as_ref().cloned().ok_or("Database not initialized")?
    };
    db.ensure_global_scope().await.map_err(|e| e.to_string())?;
    let stop_signal = Arc::clone(&state.stop_signal);
    let recording = Arc::clone(&state.recording);
    let segments = Arc::clone(&state.segments);
    let recorded_audio = Arc::clone(&state.recorded_audio);
    let recognizer_arc = Arc::clone(&state.recognizer);
    let vad_arc = Arc::clone(&state.vad);
    let correction_engine = Arc::clone(&state.correction_engine);
    let app_state = Arc::new(AppState {
        recognizer: Arc::clone(&state.recognizer),
        vad: Arc::clone(&state.vad),
        recording: Arc::clone(&state.recording),
        stop_signal: Arc::clone(&state.stop_signal),
        segments: Arc::clone(&state.segments),
        recorded_audio: Arc::clone(&state.recorded_audio),
        db: Arc::clone(&state.db),
        db_writer: Arc::clone(&state.db_writer),
        correction_engine: Arc::clone(&state.correction_engine),
        start_wall_clock: Arc::clone(&state.start_wall_clock),
        start_instant: Arc::clone(&state.start_instant),
        init_status: Arc::clone(&state.init_status),
        init_error: Arc::clone(&state.init_error),
        num_threads: Arc::clone(&state.num_threads),
        next_realtime_segment_id: Arc::clone(&state.next_realtime_segment_id),
        next_revision: Arc::clone(&state.next_revision),
        settings: Arc::clone(&state.settings),
        llm_settings: Arc::clone(&state.llm_settings),
        quality_filter_config: Arc::clone(&state.quality_filter_config),
        llm_models_cache: Arc::clone(&state.llm_models_cache),
        selected_device: Arc::clone(&state.selected_device),
        app_handle: Arc::new(RwLock::new(Some(app.clone()))),
    });
    let db_writer = state.db_writer.as_ref().clone();
    let next_realtime_segment_id = Arc::clone(&state.next_realtime_segment_id);
    let next_revision = Arc::clone(&state.next_revision);
    let anchor = RecordingAnchor {
        base_wall: now,
        audio_offset,
    };
    let selected_device = read_lock(&state.selected_device).clone();

    let db_writer_for_run = db_writer.clone();
    tauri::async_runtime::spawn(async move {
        let join = tauri::async_runtime::spawn_blocking(move || {
            run_recording(
                recognizer,
                vad,
                RecordingRuntime {
                    stop_signal: &stop_signal,
                    segments: &segments,
                    next_realtime_segment_id,
                    next_revision,
                    correction_engine: &correction_engine,
                    app_state: &app_state,
                    app_handle: &app,
                    db_writer: db_writer_for_run.clone(),
                    recorded_audio: &recorded_audio,
                    anchor: &anchor,
                    selected_device: selected_device.as_deref(),
                },
            )
        });

        match join.await {
            Ok(Ok((rec, v))) => {
                {
                    let mut r = write_lock(&recognizer_arc);
                    *r = Some(rec);
                }
                {
                    let mut va = write_lock(&vad_arc);
                    *va = Some(v);
                }
            }
            Ok(Err(err)) => {
                error!("[recording task] error: {err}");
            }
            Err(err) => {
                error!("[recording task] join failed: {err}");
            }
        }

        recording.store(false, Ordering::SeqCst);
        let _ = db_writer.try_send(DbEvent::TouchGlobalScopeEnd);
        info!("[recording task] stopped");
    });

    Ok(())
}

fn run_recording(
    recognizer: OfflineRecognizer,
    vad: VoiceActivityDetector,
    runtime: RecordingRuntime<'_>,
) -> Result<(OfflineRecognizer, VoiceActivityDetector), String> {
    let host = cpal::default_host();
    let device = if let Some(name) = runtime.selected_device {
        host.input_devices()
            .map_err(|e| format!("Cannot enumerate devices: {e}"))?
            .find(|d| d.name().ok().as_deref() == Some(name))
            .ok_or_else(|| format!("Device not found: {name}"))?
    } else {
        host.default_input_device().ok_or("No default input device")?
    };

    info!("[recording] device: {:?}", device.name().unwrap_or_default());

    let supported = device
        .default_input_config()
        .map_err(|e| format!("No input config: {e}"))?;
    let mic_sample_rate = supported.sample_rate().0 as i32;

    let resampler = if mic_sample_rate != SAMPLE_RATE as i32 {
        Some(
            LinearResampler::create(mic_sample_rate, SAMPLE_RATE as i32)
                .ok_or_else(|| format!("Failed to create resampler for {mic_sample_rate} Hz"))?,
        )
    } else {
        None
    };

    let (tx, rx) = mpsc::channel::<Vec<f32>>();
    let received_audio = Arc::new(AtomicBool::new(false));
    let stream = build_input_stream(&device, tx, Arc::clone(&received_audio))?;
    stream.play().map_err(|e| format!("Stream play: {e}"))?;
    info!("[recording] input stream started, waiting for microphone samples");

    vad.reset();
    let window_size: usize = 512;
    let mut vad_buf: Vec<f32> = Vec::new();
    let wait_started_at = Instant::now();
    let mut warned_no_audio = false;

    loop {
        if runtime.stop_signal.load(Ordering::Relaxed) {
            break;
        }

        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(samples) => {
                let pcm = if let Some(ref resamp) = resampler {
                    resamp.resample(&samples, false)
                } else {
                    samples
                };

                {
                    let mut audio = write_lock(runtime.recorded_audio);
                    audio.push_samples(&pcm);
                }

                vad_buf.extend_from_slice(&pcm);

                while vad_buf.len() >= window_size {
                    vad.accept_waveform(&vad_buf[..window_size]);
                    vad_buf.drain(..window_size);

                    while let Some(segment) = vad.front() {
                        let recognize_ctx = RecognizeContext {
                            segments: runtime.segments.clone(),
                            next_realtime_segment_id: runtime.next_realtime_segment_id.clone(),
                            next_revision: runtime.next_revision.clone(),
                            correction_engine: runtime.correction_engine.clone(),
                            state: runtime.app_state.clone(),
                            app_handle: runtime.app_handle.clone(),
                            db_writer: runtime.db_writer.clone(),
                            base_wall: runtime.anchor.base_wall,
                            audio_offset_samples: runtime.anchor.audio_offset,
                        };
                        recognize_segment(&recognizer, &segment, recognize_ctx);
                        vad.pop();
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if !warned_no_audio
                    && !received_audio.load(Ordering::Relaxed)
                    && wait_started_at.elapsed() >= Duration::from_secs(3)
                {
                    warned_no_audio = true;
                    warn!("[recording] no microphone samples received within 3 seconds");
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                info!("[recording] audio channel disconnected");
                break;
            }
        }
    }

    drop(stream);

    if !vad_buf.is_empty() {
        vad_buf.resize(window_size, 0.0);
        vad.accept_waveform(&vad_buf[..window_size]);
    }

    vad.flush();
    while let Some(segment) = vad.front() {
        let recognize_ctx = RecognizeContext {
            segments: runtime.segments.clone(),
            next_realtime_segment_id: runtime.next_realtime_segment_id.clone(),
            next_revision: runtime.next_revision.clone(),
            correction_engine: runtime.correction_engine.clone(),
            state: runtime.app_state.clone(),
            app_handle: runtime.app_handle.clone(),
            db_writer: runtime.db_writer.clone(),
            base_wall: runtime.anchor.base_wall,
            audio_offset_samples: runtime.anchor.audio_offset,
        };
        recognize_segment(&recognizer, &segment, recognize_ctx);
        vad.pop();
    }

    let seg_count = read_lock(runtime.segments).len();
    info!("[recording] flushed, total segments: {seg_count}");

    Ok((recognizer, vad))
}

fn build_input_stream(
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
                        .map(|frame| {
                            let sum: f32 = frame.iter().copied().sum();
                            sum / channels as f32
                        })
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
                            let sum: f32 = frame.iter().map(|&s| s as f32 / i16::MAX as f32).sum();
                            sum / channels as f32
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
                            let sum: f32 = frame.iter().map(|&s| (s as f32 - 32768.0) / 32768.0).sum();
                            sum / channels as f32
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

fn recognize_segment(recognizer: &OfflineRecognizer, segment: &sherpa_onnx::SpeechSegment, ctx: RecognizeContext) {
    let samples = segment.samples();
    let duration = samples.len() as f32 / SAMPLE_RATE as f32;
    if duration < 0.1 {
        return;
    }
    let vad_start = segment.start() as f32 / SAMPLE_RATE as f32;
    info!("[vad] detected speech segment start={vad_start:.2}s duration={duration:.2}s");
    let stream = recognizer.create_stream();
    stream.accept_waveform(16000, samples);
    recognizer.decode(&stream);
    tokio::spawn(async move {
        recognize_segment_task(stream, vad_start, duration, ctx).await;
    });
}
async fn recognize_segment_task(stream: OfflineStream, vad_start: f32, duration: f32, ctx: RecognizeContext) {
    let offset_secs = ctx.audio_offset_samples as f32 / SAMPLE_RATE as f32;
    let rel_start = offset_secs + vad_start;
    let rel_end = rel_start + duration;
    if let Some(r) = stream.get_result() {
        let text_raw = r.text.trim().to_string();
        info!(
            "[asr] decoded segment start={vad_start:.2}s duration={duration:.2}s text={:?}",
            text_raw
        );

        // Filter out results that contain Japanese Hiragana or Katakana,
        // as the model often misidentifies silence/noise as Japanese.
        let has_japanese = text_raw.chars().any(|c| {
            ('\u{3040}'..='\u{309f}').contains(&c) || // Hiragana
            ('\u{30a0}'..='\u{30ff}').contains(&c) // Katakana
        });

        // Filter out results that consist only of punctuation or whitespace (including CJK punctuation).
        let is_meaningless = text_raw.chars().all(|c| {
            c.is_ascii_punctuation()
                || c.is_ascii_whitespace()
                || matches!(
                    c,
                    '.' | '。'
                        | '，'
                        | '？'
                        | '！'
                        | '…'
                        | '—'
                        | '·'
                        | '、'
                        | '；'
                        | '：'
                        | '“'
                        | '”'
                        | '‘'
                        | '’'
                        | '（'
                        | '）'
                        | '【'
                        | '】'
                        | '《'
                        | '》'
                )
        });

        // Filter out common filler words or hallucinations like "Yeah."
        let is_filler = matches!(text_raw.to_lowercase().as_str(), "yeah." | "yeah" | "yeah!");

        if !text_raw.is_empty() && !has_japanese && !is_meaningless && !is_filler {
            let text_corrected = ctx.correction_engine.apply(&text_raw);
            let revision = ctx.next_revision.fetch_add(1, Ordering::Relaxed) as i64;
            let wall_start = ctx.base_wall + chrono::Duration::milliseconds((vad_start * 1000.0) as i64);
            let wall_end = ctx.base_wall + chrono::Duration::milliseconds(((vad_start + duration) * 1000.0) as i64);

            let wall_start_fmt = wall_start.format("%Y-%m-%d %H:%M:%S").to_string();
            let wall_end_fmt = wall_end.format("%Y-%m-%d %H:%M:%S").to_string();
            let new_segment = SegmentResult {
                segment_id: ctx.next_realtime_segment_id.fetch_add(1, Ordering::Relaxed),
                revision,
                update_type: crate::SegmentUpdateType::Append,
                start: rel_start,
                end: rel_end,
                wall_start: wall_start_fmt.clone(),
                wall_end: wall_end_fmt.clone(),
                text: text_corrected.clone(),
                text_optimized: None,
                text_english: None,
                optimize_status: "pending".to_string(),
                translate_status: "blocked".to_string(),
                finalization_check_state: "not_ready".to_string(),
                is_discarded: false,
                discard_reason: None,
                discard_source: None,
                discard_confidence: None,
                quality_check_status: "pending".to_string(),
                text_raw: text_raw.clone(),
            };

            let (db_segment_id, llm_input_text) = {
                let mut segs = write_lock(&ctx.segments);
                if merge_segment_in_place(&mut segs, &new_segment) {
                    segs.last()
                        .map(|s| (s.segment_id, s.text.clone()))
                        .unwrap_or((new_segment.segment_id, new_segment.text.clone()))
                } else {
                    let id = (new_segment.segment_id, new_segment.text.clone());
                    segs.push(new_segment);
                    id
                }
            };

            // let llm_input_text = ctx
            //     .segments
            //     .read()
            //     .map(|guard| guard.last().map(|seg| seg.text.clone()))
            //     .unwrap_or_else(|poisoned| poisoned.into_inner().last().map(|seg| seg.text.clone()))
            //     .filter(|t| !t.trim().is_empty())
            //     .unwrap_or_else(|| text_corrected.clone());

            let event = DbEvent::InsertSegment {
                segment: NewSegment {
                    segment_id: db_segment_id,
                    revision,
                    start_sec: rel_start,
                    end_sec: rel_end,
                    wall_start: wall_start_fmt,
                    wall_end: wall_end_fmt,
                    text_raw,
                },
            };
            if let Err(err) = ctx.db_writer.try_send(event) {
                if matches!(err, TrySendError::Full(_)) {
                    warn!("[db-worker] queue full, dropping segment event");
                } else {
                    warn!(
                        "[db-worker] failed to enqueue segment segment_id={}, revision={}, err={}",
                        db_segment_id, revision, err
                    );
                }
            } else {
                debug!(
                    "[db-worker] enqueued segment segment_id={}, revision={}",
                    db_segment_id, revision
                );
            }

            spawn_llm_postprocess_task_v2(
                ctx.db_writer.clone(),
                ctx.state.clone(),
                ctx.app_handle.clone(),
                revision,
                llm_input_text,
            );
        }
    }
}

fn spawn_llm_postprocess_task_v2(
    writer: SyncSender<DbEvent>,
    state: Arc<AppState>,
    app_handle: tauri::AppHandle,
    revision: i64,
    llm_input_text: String,
) {
    tauri::async_runtime::spawn(async move {
        info!(
            "[llm] start postprocess revision={}, text_len={}",
            revision,
            llm_input_text.len()
        );
        update_segment_llm_state(&state.segments, revision, Some("running"), None, None, None);
        let _ = writer.try_send(DbEvent::MarkSkippedBefore { revision });
        let _ = writer.try_send(DbEvent::MarkOptimizeRunning { revision });

        let settings = read_lock(&state.llm_settings).clone();

        if settings.selected_model.trim().is_empty() {
            error!("");
        }
        // if settings.selected_model.trim().is_empty() {
        //     match llm_list_models(&settings).await {
        //         Ok(models) => {
        //             if let Some(first) = models.into_iter().find(|m| !m.trim().is_empty()) {
        //                 warn!(
        //                     "[llm] selected_model is empty, fallback to first model={}, revision={}",
        //                     first, revision
        //                 );
        //                 let mut fallback_settings = settings.clone();
        //                 fallback_settings.selected_model = first;
        //                 perform_postprocess_and_copy(
        //                     &writer,
        //                     &state,
        //                     &app_handle,
        //                     revision,
        //                     &llm_input_text,
        //                     fallback_settings,
        //                 )
        //                 .await;
        //                 return;
        //             } else {
        //                 warn!("[llm] skip due to empty model list, revision={}", revision);
        //                 update_segment_llm_state(&state.segments, revision, Some("failed"), None, None, None);
        //                 let _ = writer.try_send(DbEvent::MarkOptimizeFailed { revision });
        //                 return;
        //             }
        //         }
        //         Err(err) => {
        //             warn!(
        //                 "[llm] skip due to empty model and list_models failed, revision={}, err={}",
        //                 revision, err
        //             );
        //             update_segment_llm_state(&state.segments, revision, Some("failed"), None, None, None);
        //             let _ = writer.try_send(DbEvent::MarkOptimizeFailed { revision });
        //             return;
        //         }
        //     }
        // }

        perform_postprocess_and_copy(&writer, &state, &app_handle, revision, &llm_input_text, settings).await;
    });
}

async fn perform_postprocess_and_copy(
    writer: &SyncSender<DbEvent>,
    state: &Arc<AppState>,
    app_handle: &tauri::AppHandle,
    revision: i64,
    llm_input_text: &str,
    settings: LlmSettings,
) {
    let optimized = match optimize_text(&settings, llm_input_text).await {
        Ok(v) => v,
        Err(err) => {
            error!("llm postprocess failed: {}", err);
            update_segment_llm_state(&state.segments, revision, Some("failed"), None, None, None);
            let _ = writer.try_send(DbEvent::MarkOptimizeFailed { revision });
            return;
        }
    };

    let latest_revision = state.next_revision.load(Ordering::Relaxed) as i64 - 1;
    if revision < latest_revision {
        info!(
            "[llm] revision skipped as stale, revision={}, latest_revision={}",
            revision, latest_revision
        );
        update_segment_llm_state(&state.segments, revision, Some("failed"), None, None, None);
        let _ = writer.try_send(DbEvent::MarkSkipped { revision });
        return;
    }

    let optimized_for_memory = optimized.clone();
    let result = DbEvent::SaveOptimizeResult {
        revision,
        text_optimized: optimized,
    };
    if writer.try_send(result).is_err() {
        update_segment_llm_state(&state.segments, revision, Some("failed"), None, None, None);
        let _ = writer.try_send(DbEvent::MarkOptimizeFailed { revision });
        return;
    }
    info!(
        "[llm] optimize done, revision={}, optimized_len={}",
        revision,
        optimized_for_memory.len()
    );
    let _ = writer.try_send(DbEvent::MarkOptimizeSuccess { revision });
    let _ = writer.try_send(DbEvent::MarkTranslatePending { revision });
    update_segment_llm_state(
        &state.segments,
        revision,
        Some("success"),
        Some("pending"),
        Some(optimized_for_memory.clone()),
        None,
    );

    maybe_copy_optimized_result(app_handle, &settings.auto_copy_mode, &optimized_for_memory, revision);

    let _ = writer.try_send(DbEvent::MarkTranslateRunning { revision });
    update_segment_llm_state(&state.segments, revision, None, Some("running"), None, None);

    let english = match translate_text(&settings, &optimized_for_memory).await {
        Ok(v) => v,
        Err(err) => {
            error!("llm translate failed: {}", err);
            let _ = writer.try_send(DbEvent::MarkTranslateFailed { revision });
            update_segment_llm_state(&state.segments, revision, None, Some("failed"), None, None);
            return;
        }
    };

    let result = DbEvent::SaveTranslateResult {
        revision,
        text_english: english.clone(),
    };
    if writer.try_send(result).is_err() {
        let _ = writer.try_send(DbEvent::MarkTranslateFailed { revision });
        update_segment_llm_state(&state.segments, revision, None, Some("failed"), None, None);
        return;
    }

    info!(
        "[llm] translate done, revision={}, english_len={}",
        revision,
        english.len()
    );
    update_segment_llm_state(
        &state.segments,
        revision,
        None,
        Some("success"),
        None,
        Some(english.clone()),
    );

    maybe_copy_translated_result(app_handle, &settings.auto_copy_mode, &english, revision);

    schedule_finalization_check(state, revision);
}

fn maybe_copy_optimized_result(
    app_handle: &tauri::AppHandle,
    auto_copy_mode: &AutoCopyMode,
    optimized: &str,
    revision: i64,
) {
    if !matches!(auto_copy_mode, &AutoCopyMode::OptimizedZh) {
        return;
    }

    if let Err(err) = app_handle.clipboard().write_text(optimized) {
        error!("copy 优化中文 to clipboard failed: {}", err);
    } else {
        info!("[llm] auto copy done, revision={}, mode=优化中文", revision);
    }
}

fn maybe_copy_translated_result(
    app_handle: &tauri::AppHandle,
    auto_copy_mode: &AutoCopyMode,
    english: &str,
    revision: i64,
) {
    if !matches!(auto_copy_mode, &AutoCopyMode::English) {
        return;
    }

    if let Err(err) = app_handle.clipboard().write_text(english) {
        error!("copy 英文 to clipboard failed: {}", err);
    } else {
        info!("[llm] auto copy done, revision={}, mode=英文", revision);
    }
}

fn schedule_finalization_check(state: &Arc<AppState>, revision: i64) {
    let segments = Arc::clone(&state.segments);
    let db_writer = Arc::clone(&state.db_writer);
    let llm_settings = Arc::clone(&state.llm_settings);
    let quality_filter_config = Arc::clone(&state.quality_filter_config);
    let db = Arc::clone(&state.db);
    let app_handle = {
        let guard = read_lock(&state.app_handle);
        guard.clone()
    };

    tauri::async_runtime::spawn(async move {
        let config = read_lock(&quality_filter_config).clone();
        tokio::time::sleep(Duration::from_millis(FINALIZE_SILENCE_MS)).await;
        if !can_start_finalization_check(&segments, revision) {
            return;
        }

        // If quality filter is disabled, skip discard logic but keep logging
        if !config.enabled {
            info!("[finalization] quality filter disabled, skipping discard for revision={revision}");
            set_segment_finalization_state(&segments, revision, "kept");
            set_segment_discard_state(
                &segments,
                revision,
                false,
                Some("质量过滤已禁用".to_string()),
                Some("system".to_string()),
                None,
                "kept",
            );
            let _ = db_writer.try_send(DbEvent::UpdateDiscardResult {
                revision,
                is_discarded: false,
                discard_reason: Some("质量过滤已禁用".to_string()),
                discard_source: Some("system".to_string()),
                discard_confidence: None,
                quality_check_status: "kept".to_string(),
            });
            return;
        }

        set_segment_finalization_state(&segments, revision, "ready");
        set_segment_finalization_state(&segments, revision, "checking");

        // Step 1: Load segment data from DB
        let db_instance = {
            let db_guard = mutex_lock(&db);
            db_guard.clone()
        };
        let segment_row = match db_instance {
            Some(db) => match db.get_segment_by_revision(revision).await {
                Ok(row) => row,
                Err(e) => {
                    error!("[finalization] failed to get segment revision={revision}: {e}");
                    set_segment_finalization_state(&segments, revision, "check_failed");
                    let _ = db_writer.try_send(DbEvent::UpdateDiscardResult {
                        revision,
                        is_discarded: false,
                        discard_reason: Some("终态判定读取分段失败".to_string()),
                        discard_source: Some("system".to_string()),
                        discard_confidence: None,
                        quality_check_status: "check_failed".to_string(),
                    });
                    return;
                }
            },
            None => {
                error!("[finalization] database not available");
                set_segment_finalization_state(&segments, revision, "check_failed");
                let _ = db_writer.try_send(DbEvent::UpdateDiscardResult {
                    revision,
                    is_discarded: false,
                    discard_reason: Some("终态判定数据库不可用".to_string()),
                    discard_source: Some("system".to_string()),
                    discard_confidence: None,
                    quality_check_status: "check_failed".to_string(),
                });
                return;
            }
        };

        let seg = match segment_row {
            Some(s) => s,
            None => {
                warn!("[finalization] segment not found, revision={revision}");
                set_segment_finalization_state(&segments, revision, "check_failed");
                let _ = db_writer.try_send(DbEvent::UpdateDiscardResult {
                    revision,
                    is_discarded: false,
                    discard_reason: Some("终态判定未找到分段".to_string()),
                    discard_source: Some("system".to_string()),
                    discard_confidence: None,
                    quality_check_status: "check_failed".to_string(),
                });
                return;
            }
        };

        // Step 2: Rule-based discard (lightweight, no LLM)
        let text_to_check = seg.text_raw.clone();
        if check_discard_rules(&text_to_check, &config) {
            info!("[finalization] rule-based DISCARD, revision={revision}, text={text_to_check}");
            set_segment_finalization_state(&segments, revision, "discarded");
            set_segment_discard_state(
                &segments,
                revision,
                true,
                Some("规则层判定：填充词/低信息".to_string()),
                Some("rule".to_string()),
                None,
                "discarded",
            );
            let _ = db_writer.try_send(DbEvent::UpdateDiscardResult {
                revision,
                is_discarded: true,
                discard_reason: Some("规则层判定：填充词/低信息".to_string()),
                discard_source: Some("rule".to_string()),
                discard_confidence: None,
                quality_check_status: "discarded".to_string(),
            });
            // Send frontend event (Plan 3)
            if let Some(app_handle) = &app_handle {
                let _ = app_handle.emit(
                    "segment_discarded",
                    serde_json::json!({
                        "revision": revision,
                        "segment_id": seg.segment_id,
                        "decision": "DISCARD",
                        "reason": "规则层判定：填充词/低信息",
                        "source": "rule",
                        "confidence": null,
                        "occurred_at_ms": chrono::Utc::now().timestamp_millis(),
                    }),
                );
            }
            return;
        }

        // Step 3: LLM judgment
        set_segment_finalization_state(&segments, revision, "llm_checking");
        let settings = read_lock(&llm_settings).clone();
        let judgment_input = JudgmentInput {
            text_raw: seg.text_raw.clone(),
            text_optimized: seg.text_optimized.clone(),
            text_english: seg.text_english.clone(),
        };

        let judgment_result = match judge_discard(&settings, &config, &judgment_input).await {
            Ok(r) => r,
            Err(e) => {
                error!("[finalization] LLM judgment failed, revision={revision}: {e}");
                set_segment_finalization_state(&segments, revision, "check_failed");
                let _ = db_writer.try_send(DbEvent::UpdateDiscardResult {
                    revision,
                    is_discarded: false,
                    discard_reason: Some(format!("终态判定模型调用失败: {e}")),
                    discard_source: Some("llm".to_string()),
                    discard_confidence: None,
                    quality_check_status: "check_failed".to_string(),
                });
                return;
            }
        };

        // Step 4: Evaluate and apply result
        let should_discard = evaluate_judgment(&judgment_result, &config);
        if should_discard {
            info!(
                "[finalization] LLM DISCARD, revision={revision}, confidence={}, reason={}",
                judgment_result.confidence, judgment_result.reason
            );
            set_segment_finalization_state(&segments, revision, "discarded");
            set_segment_discard_state(
                &segments,
                revision,
                true,
                Some(judgment_result.reason.clone()),
                Some("llm".to_string()),
                Some(judgment_result.confidence),
                "discarded",
            );
            let _ = db_writer.try_send(DbEvent::UpdateDiscardResult {
                revision,
                is_discarded: true,
                discard_reason: Some(judgment_result.reason.clone()),
                discard_source: Some("llm".to_string()),
                discard_confidence: Some(judgment_result.confidence),
                quality_check_status: "discarded".to_string(),
            });
            // Send frontend event (Plan 3)
            if let Some(app_handle) = &app_handle {
                let _ = app_handle.emit(
                    "segment_discarded",
                    serde_json::json!({
                        "revision": revision,
                        "segment_id": seg.segment_id,
                        "decision": "DISCARD",
                        "reason": judgment_result.reason,
                        "source": "llm",
                        "confidence": judgment_result.confidence,
                        "occurred_at_ms": chrono::Utc::now().timestamp_millis(),
                    }),
                );
            }
        } else {
            info!(
                "[finalization] LLM KEEP, revision={revision}, confidence={}, reason={}",
                judgment_result.confidence, judgment_result.reason
            );
            set_segment_finalization_state(&segments, revision, "kept");
            set_segment_discard_state(
                &segments,
                revision,
                false,
                None,
                Some("llm".to_string()),
                Some(judgment_result.confidence),
                "kept",
            );
            let _ = db_writer.try_send(DbEvent::UpdateDiscardResult {
                revision,
                is_discarded: false,
                discard_reason: None,
                discard_source: Some("llm".to_string()),
                discard_confidence: Some(judgment_result.confidence),
                quality_check_status: "kept".to_string(),
            });
        }
    });
}

#[tauri::command]
pub fn stop_recording(state: tauri::State<'_, AppState>) {
    info!("[stop_recording]");
    info!("[stop_recording] signalling stop");
    state.stop_signal.store(true, Ordering::Relaxed);
}

#[tauri::command]
pub fn clear_results(state: tauri::State<'_, AppState>) -> Result<(), String> {
    info!("[clear_results]");
    if state.recording.load(Ordering::SeqCst) {
        return Err("Cannot clear while recording".to_string());
    }
    write_lock(&state.segments).clear();
    write_lock(&state.recorded_audio).clear();
    *write_lock(&state.start_wall_clock) = None;
    *write_lock(&state.start_instant) = None;
    info!("[clear_results] cleared all segments and audio");
    Ok(())
}

#[tauri::command]
pub fn get_recording_state(state: tauri::State<'_, AppState>) -> Result<RecordingState, String> {
    let recording = state.recording.load(Ordering::Relaxed);
    let segments = read_lock(&state.segments)
        .iter()
        .filter(|seg| !seg.is_discarded)
        .cloned()
        .collect();
    let elapsed_secs = 0.0;
    let (audio_window_start_sec, audio_window_end_sec) = {
        let audio = read_lock(&state.recorded_audio);
        (
            audio.global_start_sample() as f32 / SAMPLE_RATE as f32,
            audio.global_end_sample() as f32 / SAMPLE_RATE as f32,
        )
    };
    Ok(RecordingState {
        recording,
        segments,
        elapsed_secs,
        audio_window_start_sec,
        audio_window_end_sec,
    })
}
