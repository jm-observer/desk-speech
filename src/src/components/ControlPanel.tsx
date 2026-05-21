import React from 'react';
import { RecordCard } from './RecordCard';
import { Dropdown } from './ui/Dropdown';
import { Switch } from './ui/Switch';
import { Button } from './ui/Button';
import { Icon } from './ui/Icon';
import { StatusChip } from './StatusChip';
import type { AsrLanguage, AutoCopyMode } from '../api/tauri-client';

interface ControlPanelProps {
  status: string;
  devices: { label: string; value: string }[];
  selectedDevice: string;
  onDeviceChange: (val: string) => void;
  showEnglish: boolean;
  onShowEnglishChange: (val: boolean) => void;
  autoRecordingEnabled: boolean;
  onAutoRecordingEnabledChange: (val: boolean) => void;
  onStart: () => void;
  onStop: () => void;
  onRetry: () => void;
  errorMessage: string;
  onClear: () => void;
  asrLanguage: AsrLanguage;
  onAsrLanguageChange: (val: AsrLanguage) => void;
  autoCopyMode: AutoCopyMode;
  onAutoCopyModeChange: (val: AutoCopyMode) => void;
  onToggleMode: () => void;
  disabled?: boolean;
}

const LANGUAGE_OPTIONS: { label: string; value: AsrLanguage }[] = [
  { label: '自动检测', value: '' },
  { label: '中文', value: 'zh' },
  { label: '英文', value: 'en' },
  { label: '日文', value: 'ja' },
  { label: '韩文', value: 'ko' },
  { label: '粤语', value: 'yue' },
];

const AUTO_COPY_OPTIONS: { label: string; value: AutoCopyMode }[] = [
  { label: '关闭', value: 'off' },
  { label: '复制中文优化', value: 'optimized_zh' },
  { label: '复制英文翻译', value: 'english' },
];

export const ControlPanel: React.FC<ControlPanelProps> = ({
  status,
  devices,
  selectedDevice,
  onDeviceChange,
  showEnglish,
  onShowEnglishChange,
  autoRecordingEnabled,
  onAutoRecordingEnabledChange,
  onStart,
  onStop,
  onRetry,
  errorMessage,
  onClear,
  asrLanguage,
  onAsrLanguageChange,
  autoCopyMode,
  onAutoCopyModeChange,
  onToggleMode,
  disabled,
}) => {
  return (
    <aside className="w-80 shrink-0 h-[calc(100%-24px)] my-3 mx-3 flex flex-col bg-[var(--bg-app)] border border-[var(--line)] rounded-[16px] px-6 py-4 gap-4 overflow-y-auto drag-region">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2.5">
          <div className="w-8 h-8 rounded-[9px] bg-gradient-to-br from-[var(--primary)] to-[var(--primary-deep)] shadow-sm flex items-center justify-center text-white">
            <Icon name="logo" size={20} stroke={2} />
          </div>
          <div className="flex flex-col">
            <h1 className="text-[14.5px] font-semibold text-[var(--ink)] leading-tight">StreamSpeech</h1>
            <span className="text-[11px] text-[var(--ink-4)]">远程语音识别 · v1.13.0</span>
          </div>
        </div>
        <Button variant="ghost" size="icon" onClick={onToggleMode} className="rounded-full w-7 h-7">
          <Icon name="search" size={14} className="text-[var(--ink-3)]" />
        </Button>
      </div>

      <StatusChip status={status} />

      <Dropdown
        label="输入设备"
        icon="mic"
        options={devices}
        value={selectedDevice}
        onChange={onDeviceChange}
        disabled={status === 'recording'}
      />

      <Dropdown
        label="识别语言"
        icon="languages"
        options={LANGUAGE_OPTIONS}
        value={asrLanguage}
        onChange={(v) => onAsrLanguageChange(v as AsrLanguage)}
        disabled={status === 'recording' || disabled}
      />

      <Dropdown
        label="自动复制"
        icon="copy"
        options={AUTO_COPY_OPTIONS}
        value={autoCopyMode}
        onChange={(v) => onAutoCopyModeChange(v as AutoCopyMode)}
        disabled={disabled}
      />

      <RecordCard
        status={status}
        onStart={onStart}
        onStop={onStop}
        onRetry={onRetry}
        errorMessage={errorMessage}
        disabled={devices.length === 0 || disabled}
      />

      <div className="flex flex-col gap-4 mt-1">
        <div className="flex items-center justify-between">
          <div className="flex flex-col">
            <span className="text-[13px] font-medium text-[var(--ink-2)]">显示英文翻译</span>
            <span className="text-[11px] text-[var(--ink-4)]">LLM 同步生成对照翻译</span>
          </div>
          <Switch checked={showEnglish} onCheckedChange={onShowEnglishChange} />
        </div>
        <div className="flex items-center justify-between">
          <div className="flex flex-col">
            <span className="text-[13px] font-medium text-[var(--ink-2)]">自动录音</span>
            <span className="text-[11px] text-[var(--ink-4)]">刷新后会按当前模式自动检测并尝试启动</span>
          </div>
          <Switch checked={autoRecordingEnabled} onCheckedChange={onAutoRecordingEnabledChange} disabled={disabled} />
        </div>
      </div>

      <div className="flex-1" />

      <div className="pt-4 border-t border-[var(--line)]">
        <Button variant="soft" size="sm" className="w-full h-9 rounded-lg text-xs" onClick={onClear} disabled={status === 'recording' || disabled}>
          清空结果
        </Button>
        <p className="mt-3 text-[11px] leading-relaxed text-[var(--ink-4)]">
          ASR 模型、LLM 提示词、声纹等服务端配置请打开
          <br />
          GB10 管理台 (http://&lt;server&gt;:8090/) 调整。
        </p>
      </div>
    </aside>
  );
};
