import React from 'react';
import { cn } from '../utils';
import { Waveform } from './Waveform';
import { Button } from './ui/Button';
import { Icon } from './ui/Icon';

interface RecordCardProps {
  status: string;
  onStart: () => void;
  onStop: () => void;
  disabled?: boolean;
}

export const RecordCard: React.FC<RecordCardProps> = ({ 
  status, 
  onStart, 
  onStop,
  disabled 
}) => {
  const isRecording = status === 'recording' || status === 'processing';
  const isProcessingOnly = status === 'processing';

  return (
    <div className={cn(
      "relative flex flex-col items-center p-5 pt-4 pb-6 gap-4 rounded-[18px] border border-[var(--primary-soft)] transition-all",
      "bg-gradient-to-b from-[var(--primary-softer)] to-transparent"
    )}>
      <Waveform active={isRecording} intensity={0.8} className="h-16" />
      
      <div className="flex items-center gap-2.5 h-10">
        {isRecording && (
          <div className="flex items-center gap-2">
            <div className="w-2.5 h-2.5 rounded-full bg-[var(--danger)] animate-pulse-dot" />
            <span className="text-sm font-medium text-[var(--danger)]">
              {isProcessingOnly ? '识别处理中...' : '正在录制...'}
            </span>
          </div>
        )}
      </div>

      <div className="w-full flex flex-col items-center gap-3">
        {!isRecording ? (
          <Button 
            className="w-full h-[46px] rounded-xl text-base gap-2" 
            onClick={onStart}
            disabled={disabled || status === 'initializing'}
          >
            <Icon name="mic" size={18} />
            开始录音
          </Button>
        ) : (
          <Button 
            variant="danger" 
            className="w-full h-[46px] rounded-xl text-base gap-2 bg-white" 
            onClick={onStop}
          >
            <Icon name="stop" size={18} />
            停止录音
          </Button>
        )}
        
        <div className="flex items-center gap-1.5 text-[11px] text-[var(--ink-4)]">
          <span>快捷键</span>
          <kbd className="px-1.5 py-0.5 rounded-md bg-[var(--bg-soft)] border border-[var(--line)] font-mono text-[10px]">⌘</kbd>
          <kbd className="px-1.5 py-0.5 rounded-md bg-[var(--bg-soft)] border border-[var(--line)] font-mono text-[10px]">R</kbd>
        </div>
      </div>
    </div>
  );
};
