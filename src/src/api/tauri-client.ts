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

export interface AppSettings {
  threshold: number;
  min_silence_duration: number;
  min_speech_duration: number;
  max_speech_duration: number;
  num_threads: number;
  asr_model: 'sense-voice' | 'whisper-turbo';
  asr_provider: 'cpu' | 'cuda';
  asr_language: '' | 'zh' | 'en' | 'ja' | 'ko' | 'yue';
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
  speaker?: string;
  created_at: string;
}

export interface QualityFilterConfig {
  llm_prompt_template: string;
  discard_confidence_threshold: number;
  silence_window_ms: number;
  repeat_ratio_threshold: number;
  enabled: boolean;
  version: number;
}

export interface ConfigValidationError {
  field: string;
  message: string;
}

export interface ValidationErrorsResponse {
  errors: ConfigValidationError[];
}

export interface AppVersionInfo {
  app_version: string;
  app_name: string;
  build_profile: string;
  git_commit: string | null;
  schema_version: number;
  config_schema_version: number;
  first_run_after_upgrade: boolean;
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
  listLlmModelsWithUrl: (providerUrl: string, apiKey: string) => invoke<LlmModelList>('list_llm_models_with_url', { providerUrl, apiKey }),
  listCorrectionRules: () => invoke<CorrectionRule[]>('list_correction_rules'),
  createCorrectionRule: (payload: Omit<CorrectionRule, 'id'>) => invoke('create_correction_rule', payload),
  updateCorrectionRule: (payload: CorrectionRule) => invoke('update_correction_rule', { ...payload }),
  deleteCorrectionRule: (id: number) => invoke('delete_correction_rule', { id }),
  reloadCorrectionRules: () => invoke('reload_correction_rules'),
  getQualityFilterConfig: () => invoke<QualityFilterConfig>('get_quality_filter_config'),
  saveQualityFilterConfig: (payload: Omit<QualityFilterConfig, 'version'>) => invoke('save_quality_filter_config', { payload }),
  resetQualityFilterConfig: () => invoke<QualityFilterConfig>('reset_quality_filter_config'),
  getAppVersionInfo: () => invoke<AppVersionInfo>('get_app_version_info'),
};
