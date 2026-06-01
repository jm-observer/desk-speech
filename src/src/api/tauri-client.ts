import { invoke } from '@tauri-apps/api/core';

export interface Segment {
  id: number | null;
  segment_id?: number | null;
  revision?: number;
  start: number;
  end: number;
  wall_start: string;
  wall_end: string;
  text_raw: string;
  text_optimized?: string;
  text_english?: string;
  /** Secondary recognizer transcription (only present when the dual-model
   *  comparison opt-in is on). Used for side-by-side accuracy checks; not
   *  routed through optimize/translate. */
  text_secondary?: string;
  /** Which model produced `text_secondary` (e.g. "sensevoice"). Informational. */
  secondary_kind?: string;
  speaker?: string;
  optimize_status: 'pending' | 'running' | 'success' | 'failed';
  translate_status: 'blocked' | 'pending' | 'running' | 'success' | 'failed';
}

export interface RecordingState {
  recording: boolean;
}

export interface InputDeviceInfo {
  name: string;
  is_default: boolean;
}

export interface InitStatus {
  status: number;
  error?: string;
}

/// The two real choices the desktop client makes — everything else (model,
/// prompts, vLLM, quality filtering) is owned by the GB10 orchestrator and
/// edited from the web console at http://<gb10>:8090/.
export type AsrLanguage = '' | 'zh' | 'en' | 'ja' | 'ko' | 'yue';
export type AutoCopyMode = 'off' | 'english' | 'optimized_zh';

export interface AppSettings {
  asr_language: AsrLanguage;
  auto_copy_mode: AutoCopyMode;
  /** Auto-copy stitch window in milliseconds; 0 disables short-gap merging. */
  merge_window_ms: number;
  /** Currently selected remote orchestrator WebSocket URL. */
  remote_url: string;
  /** User-added custom orchestrator URLs (the built-in default is always shown
   * in addition to these). */
  remote_url_presets: string[];
  /** Opt-in to dual-model comparison: orchestrator also runs `asr.secondary_model`
   *  on each VAD segment and the UI shows it under the primary transcription.
   *  Takes effect at the next session start (toggling mid-session needs a
   *  stop+start, like a URL change). */
  want_secondary: boolean;
  /** Play a short system "Star" sound when a segment finishes optimize +
   *  translate (paired with the tray-icon bounce). Default ON. Turn OFF if
   *  recording with speakers + mic where the chirp leaks into the next
   *  segment. Takes effect immediately. */
  notify_sound: boolean;
}

/** Built-in orchestrator URL — always shown as the first option in the
 *  connection dropdown, regardless of saved presets. Kept in sync with
 *  `settings::DEFAULT_REMOTE_URL` in the Rust backend. */
export const DEFAULT_REMOTE_URL = 'ws://192.168.0.68:8090/stream';

export interface SegmentDiscardedEvent {
  revision: number;
  segment_id: number;
  decision: 'DISCARD';
  reason: string;
  source: 'rule' | 'llm';
  confidence: number | null;
  occurred_at_ms: number;
}

export interface SegmentUpdatedEvent {
  id: number;
  segment_id: number;
  revision: number;
  start_sec: number;
  end_sec: number;
  wall_start: string;
  wall_end: string;
  text_raw: string;
  optimize_status: Segment['optimize_status'];
  translate_status: Segment['translate_status'];
  text_optimized?: string;
  text_english?: string;
  text_secondary?: string;
  secondary_kind?: string;
  speaker?: string;
  created_at: string;
}

export const TauriAPI = {
  startRecording: () => invoke('start_recording'),
  stopRecording: () => invoke('stop_recording'),
  getRecordingState: () => invoke<RecordingState>('get_recording_state'),
  fetchRemoteHistory: (limit: number) => invoke<Record<string, unknown>[]>('fetch_remote_history', { limit }),
  listDevices: () => invoke<InputDeviceInfo[]>('list_input_devices'),
  getSelectedDevice: () => invoke<string | null>('get_selected_device'),
  setInputDevice: (deviceName: string | null) => invoke('set_input_device', { deviceName }),
  getInitStatus: () => invoke<InitStatus>('get_init_status'),
  clearResults: () => invoke('clear_results'),
  copyToClipboard: (text: string) => invoke('copy_text_to_clipboard', { text }),
  getSettings: () => invoke<AppSettings>('get_settings'),
  applySettings: (newSettings: AppSettings) => invoke('apply_settings', { newSettings }),
};
