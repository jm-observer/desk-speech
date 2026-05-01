import { invoke } from '@tauri-apps/api/core';

export interface Segment {
  id: number | null;
  start: number;
  end: number;
  wall_start: string;
  wall_end: string;
  text_raw: string;
  text_optimized?: string;
  text_english?: string;
  opt_status: 'pending' | 'running' | 'done' | 'failed' | 'skipped';
}

export interface RawSegment {
  segment_id: number | null;
  start: number;
  end: number;
  wall_start: string;
  wall_end: string;
  text: string;
  text_optimized?: string;
  text_english?: string;
  opt_status?: Segment['opt_status'];
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
  prompt_template: string;
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
  listSessions: (page: number, pageSize: number) => invoke<Record<string, unknown>[]>('list_sessions', { page, pageSize }),
  listSessionSegments: (sessionId: string, page: number, pageSize: number) => invoke<Record<string, unknown>[]>('list_session_segments', { sessionId, page, pageSize }),
  tailSessionSegments: (sessionId: string, afterId: number, limit: number) => invoke<Record<string, unknown>[]>('tail_session_segments', { sessionId, afterId, limit }),
  getRecordedAudioPath: () => invoke<string>('get_recorded_audio_path'),
  saveAllAudio: (path: string) => invoke('save_all_audio', { path }),
  exportSrt: (path: string) => invoke('export_srt', { path }),
};
