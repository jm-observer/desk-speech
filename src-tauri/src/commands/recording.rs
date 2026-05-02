use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Local;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use log::{debug, error, info, warn};
use sherpa_onnx::{LinearResampler, OfflineRecognizer, VoiceActivityDetector};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::audio_buffer::SAMPLE_RATE;
use crate::db::repository::NewSegment;
use crate::db_worker::DbEvent;
use crate::llm_client::{list_models as llm_list_models, optimize_text, translate_text};
use crate::llm_settings::{AutoCopyMode, LlmSettings};
use crate::{
    merge_segment_in_place, mutex_lock, read_lock, update_segment_llm_state, write_lock, AppState, RecognizeContext,
    RecordingAnchor, RecordingRuntime, RecordingState, SegmentResult,
};

#[tauri::command]
pub fn start_recording(app: tauri::AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    info!("[start_recording]");
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

    let audio_offset = read_lock(&state.recorded_audio).global_end_sample();

    let now = Local::now();
    *write_lock(&state.start_wall_clock) = Some(now);
    *write_lock(&state.start_instant) = Some(Instant::now());

    info!("[start_recording] starting at {now}");

    let session_id = {
        let db_guard = mutex_lock(&state.db);
        let db = db_guard.as_ref().ok_or("Database not initialized")?;
        db.create_session().map_err(|e| e.to_string())?
    };
    *write_lock(&state.current_session_id) = Some(session_id.clone());
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
        current_session_id: Arc::clone(&state.current_session_id),
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
        llm_models_cache: Arc::clone(&state.llm_models_cache),
        selected_device: Arc::clone(&state.selected_device),
    });
    let db_writer = mutex_lock(&state.db_writer).as_ref().cloned();
    info!(
        "[start_recording] created session_id={}, db_writer_ready={}",
        session_id,
        db_writer.is_some()
    );
    let current_session_id = Arc::clone(&state.current_session_id);
    let next_realtime_segment_id = Arc::clone(&state.next_realtime_segment_id);
    let next_revision = Arc::clone(&state.next_revision);
    let anchor = RecordingAnchor {
        base_wall: now,
        audio_offset,
    };
    let selected_device = read_lock(&state.selected_device).clone();

    let db_writer_for_run = db_writer.clone();
    let session_id_for_run = session_id.clone();
    tauri::async_runtime::spawn(async move {
        let join = tauri::async_runtime::spawn_blocking(move || {
            run_recording(
                recognizer,
                vad,
                RecordingRuntime {
                    stop_signal: &stop_signal,
                    segments: &segments,
                    next_realtime_segment_id: &next_realtime_segment_id,
                    next_revision: &next_revision,
                    correction_engine: &correction_engine,
                    app_state: &app_state,
                    app_handle: &app,
                    db_writer: db_writer_for_run.as_ref(),
                    session_id: &session_id_for_run,
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
        if let Some(writer) = db_writer.as_ref() {
            let _ = writer.try_send(DbEvent::CloseSession {
                session_id: session_id.clone(),
            });
        }
        {
            let mut guard = write_lock(&current_session_id);
            *guard = None;
        }
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
    let stream = build_input_stream(&device, tx)?;
    stream.play().map_err(|e| format!("Stream play: {e}"))?;

    vad.reset();
    let window_size: usize = 512;
    let mut vad_buf: Vec<f32> = Vec::new();

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
                            segments: runtime.segments,
                            next_realtime_segment_id: runtime.next_realtime_segment_id,
                            next_revision: runtime.next_revision,
                            correction_engine: runtime.correction_engine,
                            state: runtime.app_state,
                            app_handle: runtime.app_handle,
                            session_id: runtime.session_id,
                            db_writer: runtime.db_writer,
                            base_wall: &runtime.anchor.base_wall,
                            audio_offset_samples: runtime.anchor.audio_offset,
                        };
                        recognize_segment(&recognizer, &segment, &recognize_ctx);
                        vad.pop();
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
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
            segments: runtime.segments,
            next_realtime_segment_id: runtime.next_realtime_segment_id,
            next_revision: runtime.next_revision,
            correction_engine: runtime.correction_engine,
            state: runtime.app_state,
            app_handle: runtime.app_handle,
            session_id: runtime.session_id,
            db_writer: runtime.db_writer,
            base_wall: &runtime.anchor.base_wall,
            audio_offset_samples: runtime.anchor.audio_offset,
        };
        recognize_segment(&recognizer, &segment, &recognize_ctx);
        vad.pop();
    }

    let seg_count = read_lock(runtime.segments).len();
    info!("[recording] flushed, total segments: {seg_count}");

    Ok((recognizer, vad))
}

fn build_input_stream(device: &cpal::Device, tx: mpsc::Sender<Vec<f32>>) -> Result<cpal::Stream, String> {
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

fn recognize_segment(recognizer: &OfflineRecognizer, segment: &sherpa_onnx::SpeechSegment, ctx: &RecognizeContext<'_>) {
    let samples = segment.samples();
    let duration = samples.len() as f32 / SAMPLE_RATE as f32;
    if duration < 0.1 {
        return;
    }

    let vad_start = segment.start() as f32 / SAMPLE_RATE as f32;
    let offset_secs = ctx.audio_offset_samples as f32 / SAMPLE_RATE as f32;
    let rel_start = offset_secs + vad_start;
    let rel_end = rel_start + duration;

    let stream = recognizer.create_stream();
    stream.accept_waveform(16000, samples);
    recognizer.decode(&stream);

    if let Some(r) = stream.get_result() {
        let text_raw = r.text.trim().to_string();
        if !text_raw.is_empty()
            && !text_raw
                .chars()
                .all(|c| c.is_ascii_punctuation() || c.is_ascii_whitespace())
        {
            let text_corrected = ctx.correction_engine.apply(&text_raw);
            let revision = ctx.next_revision.fetch_add(1, Ordering::Relaxed) as i64;
            let wall_start = *ctx.base_wall + chrono::Duration::milliseconds((vad_start * 1000.0) as i64);
            let wall_end = *ctx.base_wall + chrono::Duration::milliseconds(((vad_start + duration) * 1000.0) as i64);

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
            };

            let db_segment_id = {
                let mut segs = write_lock(ctx.segments);
                if merge_segment_in_place(&mut segs, &new_segment) {
                    segs.last().map(|s| s.segment_id).unwrap_or(new_segment.segment_id)
                } else {
                    let id = new_segment.segment_id;
                    segs.push(new_segment);
                    id
                }
            };

            let llm_input_text = ctx
                .segments
                .read()
                .map(|guard| guard.last().map(|seg| seg.text.clone()))
                .unwrap_or_else(|poisoned| poisoned.into_inner().last().map(|seg| seg.text.clone()))
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| text_corrected.clone());

            if let Some(writer) = ctx.db_writer {
                let event = DbEvent::InsertSegment {
                    segment: NewSegment {
                        session_id: ctx.session_id.to_string(),
                        segment_id: db_segment_id,
                        revision,
                        start_sec: rel_start,
                        end_sec: rel_end,
                        wall_start: wall_start_fmt,
                        wall_end: wall_end_fmt,
                        text_raw,
                    },
                };
                if let Err(err) = writer.try_send(event) {
                    if matches!(err, TrySendError::Full(_)) {
                        warn!("[db-worker] queue full, dropping segment event");
                    } else {
                        warn!(
                            "[db-worker] failed to enqueue segment session_id={}, segment_id={}, revision={}, err={}",
                            ctx.session_id, db_segment_id, revision, err
                        );
                    }
                } else {
                    debug!(
                        "[db-worker] enqueued segment session_id={}, segment_id={}, revision={}",
                        ctx.session_id, db_segment_id, revision
                    );
                }
            } else {
                warn!("[db-worker] writer not ready, fallback to direct upsert");
                {
                    let db_guard = mutex_lock(&ctx.state.db);
                    if let Some(db) = db_guard.as_ref() {
                        let result = db.upsert_segment(NewSegment {
                            session_id: ctx.session_id.to_string(),
                            segment_id: db_segment_id,
                            revision,
                            start_sec: rel_start,
                            end_sec: rel_end,
                            wall_start: wall_start_fmt.clone(),
                            wall_end: wall_end_fmt.clone(),
                            text_raw: text_raw.clone(),
                        });
                        match result {
                            Ok(()) => info!(
                                "[db-direct] upsert ok session_id={}, segment_id={}, revision={}",
                                ctx.session_id, db_segment_id, revision
                            ),
                            Err(err) => error!(
                                "[db-direct] upsert failed session_id={}, segment_id={}, revision={}, err={}",
                                ctx.session_id, db_segment_id, revision, err
                            ),
                        }
                    }
                }
            }

            if let Some(writer) = ctx.db_writer {
                spawn_llm_postprocess_task_v2(
                    writer.clone(),
                    Arc::clone(ctx.state),
                    ctx.app_handle.clone(),
                    ctx.session_id.to_string(),
                    revision,
                    llm_input_text,
                );
            }
        }
    }
}

fn spawn_llm_postprocess_task_v2(
    writer: SyncSender<DbEvent>,
    state: Arc<AppState>,
    app_handle: tauri::AppHandle,
    session_id: String,
    revision: i64,
    llm_input_text: String,
) {
    tauri::async_runtime::spawn(async move {
        info!(
            "[llm] start postprocess session_id={}, revision={}, text_len={}",
            session_id,
            revision,
            llm_input_text.len()
        );
        update_segment_llm_state(&state.segments, revision, Some("running"), None, None, None);
        let _ = writer.try_send(DbEvent::MarkSkippedBefore {
            session_id: session_id.clone(),
            revision,
        });
        let _ = writer.try_send(DbEvent::MarkOptimizeRunning {
            session_id: session_id.clone(),
            revision,
        });

        let settings = read_lock(&state.llm_settings).clone();

        if settings.selected_model.trim().is_empty() {
            match llm_list_models(&settings).await {
                Ok(models) => {
                    if let Some(first) = models.into_iter().find(|m| !m.trim().is_empty()) {
                        warn!(
                            "[llm] selected_model is empty, fallback to first model={}, session_id={}, revision={}",
                            first, session_id, revision
                        );
                        let mut fallback_settings = settings.clone();
                        fallback_settings.selected_model = first;
                        perform_postprocess_and_copy(
                            &writer,
                            &state,
                            &app_handle,
                            &session_id,
                            revision,
                            &llm_input_text,
                            fallback_settings,
                        )
                        .await;
                        return;
                    } else {
                        warn!(
                            "[llm] skip due to empty model list, session_id={}, revision={}",
                            session_id, revision
                        );
                        update_segment_llm_state(&state.segments, revision, Some("failed"), None, None, None);
                        let _ = writer.try_send(DbEvent::MarkOptimizeFailed { session_id, revision });
                        return;
                    }
                }
                Err(err) => {
                    warn!(
                        "[llm] skip due to empty model and list_models failed, session_id={}, revision={}, err={}",
                        session_id, revision, err
                    );
                    update_segment_llm_state(&state.segments, revision, Some("failed"), None, None, None);
                    let _ = writer.try_send(DbEvent::MarkOptimizeFailed { session_id, revision });
                    return;
                }
            }
        }

        perform_postprocess_and_copy(
            &writer,
            &state,
            &app_handle,
            &session_id,
            revision,
            &llm_input_text,
            settings,
        )
        .await;
    });
}

async fn perform_postprocess_and_copy(
    writer: &SyncSender<DbEvent>,
    state: &Arc<AppState>,
    app_handle: &tauri::AppHandle,
    session_id: &str,
    revision: i64,
    llm_input_text: &str,
    settings: LlmSettings,
) {
    let optimized = match optimize_text(&settings, llm_input_text).await {
        Ok(v) => v,
        Err(err) => {
            error!("llm postprocess failed: {}", err);
            update_segment_llm_state(&state.segments, revision, Some("failed"), None, None, None);
            let _ = writer.try_send(DbEvent::MarkOptimizeFailed {
                session_id: session_id.to_string(),
                revision,
            });
            return;
        }
    };

    let latest_revision = state.next_revision.load(Ordering::Relaxed) as i64 - 1;
    if revision < latest_revision {
        info!(
            "[llm] revision skipped as stale, session_id={}, revision={}, latest_revision={}",
            session_id, revision, latest_revision
        );
        update_segment_llm_state(&state.segments, revision, Some("failed"), None, None, None);
        let _ = writer.try_send(DbEvent::MarkSkipped {
            session_id: session_id.to_string(),
            revision,
        });
        return;
    }

    let optimized_for_memory = optimized.clone();
    let result = DbEvent::SaveOptimizeResult {
        session_id: session_id.to_string(),
        revision,
        text_optimized: optimized,
    };
    if writer.try_send(result).is_err() {
        update_segment_llm_state(&state.segments, revision, Some("failed"), None, None, None);
        let _ = writer.try_send(DbEvent::MarkOptimizeFailed {
            session_id: session_id.to_string(),
            revision,
        });
        return;
    }
    info!(
        "[llm] optimize done, session_id={}, revision={}, optimized_len={}",
        session_id,
        revision,
        optimized_for_memory.len()
    );
    let _ = writer.try_send(DbEvent::MarkOptimizeSuccess {
        session_id: session_id.to_string(),
        revision,
    });
    let _ = writer.try_send(DbEvent::MarkTranslatePending {
        session_id: session_id.to_string(),
        revision,
    });
    update_segment_llm_state(
        &state.segments,
        revision,
        Some("success"),
        Some("pending"),
        Some(optimized_for_memory.clone()),
        None,
    );

    let _ = writer.try_send(DbEvent::MarkTranslateRunning {
        session_id: session_id.to_string(),
        revision,
    });
    update_segment_llm_state(&state.segments, revision, None, Some("running"), None, None);

    let english = match translate_text(&settings, &optimized_for_memory).await {
        Ok(v) => v,
        Err(err) => {
            error!("llm translate failed: {}", err);
            let _ = writer.try_send(DbEvent::MarkTranslateFailed {
                session_id: session_id.to_string(),
                revision,
            });
            update_segment_llm_state(&state.segments, revision, None, Some("failed"), None, None);
            return;
        }
    };

    let result = DbEvent::SaveTranslateResult {
        session_id: session_id.to_string(),
        revision,
        text_english: english.clone(),
    };
    if writer.try_send(result).is_err() {
        let _ = writer.try_send(DbEvent::MarkTranslateFailed {
            session_id: session_id.to_string(),
            revision,
        });
        update_segment_llm_state(&state.segments, revision, None, Some("failed"), None, None);
        return;
    }

    info!(
        "[llm] translate done, session_id={}, revision={}, english_len={}",
        session_id,
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

    let copy_text = match settings.auto_copy_mode {
        AutoCopyMode::Off => None,
        AutoCopyMode::English => Some((english, "英文")),
        AutoCopyMode::OptimizedZh => Some((optimized_for_memory, "优化中文")),
    };

    if let Some((text, mode_name)) = copy_text {
        if let Err(err) = app_handle.clipboard().write_text(text) {
            error!("copy {mode_name} to clipboard failed: {}", err);
        } else {
            info!(
                "[llm] auto copy done, session_id={}, revision={}, mode={}",
                session_id, revision, mode_name
            );
        }
    }
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
    info!("[get_recording_state]");
    let recording = state.recording.load(Ordering::Relaxed);
    let segments = read_lock(&state.segments).clone();
    let elapsed_secs = read_lock(&state.start_instant)
        .map(|instant| instant.elapsed().as_secs_f32())
        .unwrap_or(0.0);
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
