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
  optimize_status: 'pending' | 'running' | 'success' | 'failed';
  translate_status: 'blocked' | 'pending' | 'running' | 'success' | 'failed';
}

export interface RawSegment {
  segment_id: number | null;
  revision?: number;
  start: number;
  end: number;
  wall_start: string;
  wall_end: string;
  text: string;
  text_optimized?: string;
  text_english?: string;
  optimize_status?: Segment['optimize_status'];
  translate_status?: Segment['translate_status'];
}

export interface RecordingState {
  recording: boolean;
  segments: RawSegment[];
  elapsed_secs: number;
}

export interface InputDeviceInfo {
  name: string;
  is_default: boolean;
}

export interface InitStatus {
  status: number;
  error?: string;
}

export interface AppSettings {
  threshold: number;
  min_silence_duration: number;
  min_speech_duration: number;
  max_speech_duration: number;
  num_threads: number;
  provider_url: string;
  api_key: string;
  selected_model: string;
  optimize_prompt_template: string;
  translate_prompt_template: string;
  auto_copy_mode: 'off' | 'english' | 'optimized_zh';
}

export interface LlmModelList {
  models: string[];
}

export interface CorrectionRule {
  id: number;
  source: string;
  target: string;
  priority: number;
  enabled: boolean;
}

export interface SegmentDiscardedEvent {
  revision: number;
  segment_id: number;
  decision: 'DISCARD';
  reason: string;
  source: 'rule' | 'llm';
  confidence: number | null;
  occurred_at_ms: number;
}

export const TauriAPI = {
  startRecording: () => invoke('start_recording'),
  stopRecording: () => invoke('stop_recording'),
  getRecordingState: () => invoke<RecordingState>('get_recording_state'),
  listDevices: () => invoke<InputDeviceInfo[]>('list_input_devices'),
  getSelectedDevice: () => invoke<string | null>('get_selected_device'),
  setInputDevice: (deviceName: string | null) => invoke('set_input_device', { deviceName }),
  getInitStatus: () => invoke<InitStatus>('get_init_status'),
  clearResults: () => invoke('clear_results'),
  copyToClipboard: (text: string) => invoke('copy_text_to_clipboard', { text }),
  getSettings: () => invoke<AppSettings>('get_settings'),
  applySettings: (newSettings: AppSettings) => invoke('apply_settings', { newSettings }),
  listLlmModels: () => invoke<LlmModelList>('list_llm_models'),
  listCorrectionRules: () => invoke<CorrectionRule[]>('list_correction_rules'),
  createCorrectionRule: (payload: Omit<CorrectionRule, 'id'>) => invoke('create_correction_rule', payload),
  updateCorrectionRule: (payload: CorrectionRule) => invoke('update_correction_rule', { ...payload }),
  deleteCorrectionRule: (id: number) => invoke('delete_correction_rule', { id }),
  reloadCorrectionRules: () => invoke('reload_correction_rules'),
  manualOptimizeTranslate: (revision: number) => invoke('manual_optimize_translate', { revision }),
  listSegments: (page: number, pageSize: number) => invoke<Record<string, unknown>[]>('list_segments', { page, pageSize }),
  tailSegments: (afterId: number, limit: number) => invoke<Record<string, unknown>[]>('tail_segments', { afterId, limit }),
  getRecordedAudioPath: () => invoke<string>('get_recorded_audio_path'),
  saveAllAudio: (path: string) => invoke('save_all_audio', { path }),
  exportSrt: (path: string) => invoke('export_srt', { path }),
};
