import React from 'react';
import { cn, stripYear } from '../utils';
import type { Segment } from '../api/tauri-client';
import { Button } from './ui/Button';
import { Icon } from './ui/Icon';

interface SegmentCardProps {
  segment: Segment;
  showEnglish?: boolean;
  onCopy: (text: string, source: 'english' | 'optimized' | 'raw') => void;
}

export const SegmentCard: React.FC<SegmentCardProps> = ({
  segment,
  showEnglish,
  onCopy,
}) => {
  const optimizeRunning = segment.optimize_status === 'running' || segment.optimize_status === 'pending';
  const translateRunning = segment.translate_status === 'running' || segment.translate_status === 'pending';
  const isProcessing = optimizeRunning || translateRunning;
  const duration = segment.end - segment.start;
  const preferredCopyText = segment.text_english || segment.text_optimized || segment.text_raw;
  const copySource: 'english' | 'optimized' | 'raw' = segment.text_english
    ? 'english'
    : segment.text_optimized
      ? 'optimized'
      : 'raw';

  return (
    <div className={cn(
      "group relative flex flex-col p-4 px-4.5 gap-2.5 bg-[var(--bg-card)] border border-[var(--line)] rounded-[16px] shadow-[var(--shadow-sm)] transition-all animate-fade-up",
      "hover:shadow-[var(--shadow-md)] hover:border-[var(--line-strong)]"
    )}>
      <div className="flex items-center gap-3">
        <span className="px-2 py-0.5 rounded-md bg-[var(--bg-soft)] font-mono text-[11px] text-[var(--ink-2)]">
          {stripYear(segment.wall_start)} → {stripYear(segment.wall_end)}
        </span>
        <span className="text-[11px] text-[var(--ink-4)]">{duration.toFixed(1)}s</span>
        
        <div className="flex-1" />
        
        <div className="flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
          <Button variant="ghost" size="icon" className="w-7 h-7" onClick={() => onCopy(preferredCopyText, copySource)}>
            <Icon name="copy" size={14} />
          </Button>
          <Button variant="ghost" size="icon" className="w-7 h-7">
            <Icon name="download" size={14} />
          </Button>
        </div>
      </div>

      <div className="flex flex-col gap-1.5">
        <p className="text-[13px] leading-[1.7] text-[var(--ink-2)] break-words text-pretty">{segment.text_raw}</p>

        <p className={cn("text-[15px] leading-[1.7] break-words text-pretty", optimizeRunning && "text-[var(--ink-4)]")}>
          {segment.optimize_status === 'failed'
            ? '优化失败'
            : segment.text_optimized || (optimizeRunning ? '优化中...' : segment.text_raw)}
        </p>
        
        {showEnglish && (
          <p className={cn("text-[14px] leading-[1.7] break-words text-pretty", translateRunning && "text-[var(--ink-4)]")}>
            {segment.translate_status === 'failed'
              ? '翻译失败，已保留优化文本'
              : segment.text_english || (translateRunning ? '翻译中...' : '')}
          </p>
        )}
      </div>

      {isProcessing && (
        <div className="absolute top-4 right-4">
           <Icon name="refresh" size={14} className="animate-spin text-[var(--warning)]" />
        </div>
      )}
    </div>
  );
};
