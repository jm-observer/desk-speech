import React from 'react';
import { RecordCard } from './RecordCard';
import { Dropdown } from './ui/Dropdown';
import { Switch } from './ui/Switch';
import { Button } from './ui/Button';
import { Icon } from './ui/Icon';
import { StatusChip } from './StatusChip';

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
  onClear: () => void;
  onShowSettings: () => void;
  onShowRules: () => void;
  onToggleMode: () => void;
  disabled?: boolean;
}

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
  onClear,
  onShowSettings,
  onShowRules,
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
            <span className="text-[11px] text-[var(--ink-4)]">离线语音识别 · v1.13.0</span>
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

      <RecordCard
        status={status}
        onStart={onStart}
        onStop={onStop}
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

      <div className="pt-4 border-t border-[var(--line)] grid grid-cols-2 gap-2">
        <Button variant="soft" size="sm" className="w-full h-9 rounded-lg text-xs" onClick={onClear} disabled={status === 'recording' || disabled}>
          清空结果
        </Button>
        <Button variant="soft" size="sm" className="w-full h-9 rounded-lg text-xs" onClick={onShowRules} disabled={disabled}>
          词修正
        </Button>
        <Button variant="soft" size="sm" className="col-span-2 w-full h-9 rounded-lg text-xs" onClick={onShowSettings} disabled={status === 'recording' || disabled}>
          识别参数设置
        </Button>
      </div>
    </aside>
  );
};
