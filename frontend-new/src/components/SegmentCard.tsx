import React from 'react';
import { cn, stripYear } from '../utils';
import type { Segment } from '../api/tauri-client';
import { Button } from './ui/Button';
import { Icon } from './ui/Icon';

interface SegmentCardProps {
  segment: Segment;
  isActive?: boolean;
  showEnglish?: boolean;
  onSeek: (time: number) => void;
  onCopy: (text: string) => void;
}

export const SegmentCard: React.FC<SegmentCardProps> = ({
  segment,
  isActive,
  showEnglish,
  onSeek,
  onCopy,
}) => {
  const isProcessing = segment.opt_status === 'running' || segment.opt_status === 'pending';
  const duration = segment.end - segment.start;

  return (
    <div className={cn(
      "group relative flex flex-col p-4 px-4.5 gap-2.5 bg-[var(--bg-card)] border border-[var(--line)] rounded-[16px] shadow-[var(--shadow-sm)] transition-all animate-fade-up",
      "hover:shadow-[var(--shadow-md)] hover:border-[var(--line-strong)]",
      isActive && "border-[var(--primary)] shadow-[0_0_0_3px_var(--primary-soft)]"
    )}>
      <div className="flex items-center gap-3">
        <button 
          onClick={() => onSeek(segment.start)}
          className="px-2 py-0.5 rounded-md bg-[var(--bg-soft)] font-mono text-[11px] text-[var(--ink-2)] hover:bg-[var(--primary-soft)] hover:text-[var(--primary-deep)] transition-colors"
        >
          {stripYear(segment.wall_start)} → {stripYear(segment.wall_end)}
        </button>
        <span className="text-[11px] text-[var(--ink-4)]">{duration.toFixed(1)}s</span>
        
        <div className="flex-1" />
        
        <div className="flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
          <Button variant="ghost" size="icon" className="w-7 h-7" onClick={() => onCopy(segment.text_raw)}>
            <Icon name="copy" size={14} />
          </Button>
          {segment.text_english && (
            <Button variant="ghost" size="icon" className="w-7 h-7" onClick={() => onCopy(segment.text_english!)}>
              <Icon name="languages" size={14} />
            </Button>
          )}
          <Button variant="ghost" size="icon" className="w-7 h-7">
            <Icon name="download" size={14} />
          </Button>
        </div>
      </div>

      <div className="flex flex-col gap-1.5">
        <p className={cn(
          "text-[15px] leading-[1.7] text-[var(--ink)] break-words text-pretty",
          isProcessing && "text-[var(--ink-4)] animate-shimmer rounded"
        )}>
          {segment.text_optimized || segment.text_raw}
        </p>
        
        {showEnglish && segment.text_english && (
          <p className="text-[14px] leading-[1.7] text-[var(--ink-2)] break-words text-pretty">
            {segment.text_english}
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
