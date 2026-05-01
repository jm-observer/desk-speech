import React from 'react';
import { cn } from '../utils';

interface StatusChipProps {
  status: string;
}

export const StatusChip: React.FC<StatusChipProps> = ({ status }) => {
  const configs: Record<string, { label: string; bg: string; text: string; dot: string; pulse?: boolean }> = {
    idle: { label: '就绪', bg: 'var(--bg-soft)', text: 'var(--ink-2)', dot: '#7a857f' },
    initializing: { label: '模型加载中', bg: '#fef4e2', text: 'var(--warning)', dot: '#c98a2b' },
    recording: { label: '正在录音', bg: 'var(--primary-soft)', text: 'var(--primary-deep)', dot: 'var(--primary)', pulse: true },
    processing: { label: '处理中', bg: '#fef4e2', text: 'var(--warning)', dot: '#c98a2b' },
    error: { label: '异常', bg: 'var(--danger-soft)', text: 'var(--danger)', dot: 'var(--danger)' },
    finished: { label: '已完成', bg: 'var(--primary-soft)', text: 'var(--primary-deep)', dot: 'var(--primary)' },
  };

  const config = configs[status] || configs.idle;

  return (
    <div 
      className="inline-flex items-center gap-2 px-2.5 py-1 rounded-full text-[12px] font-medium transition-colors"
      style={{ backgroundColor: config.bg, color: config.text }}
    >
      <div 
        className={cn("w-1.5 h-1.5 rounded-full", config.pulse && "animate-pulse-dot")} 
        style={{ backgroundColor: config.dot }} 
      />
      {config.label}
    </div>
  );
};
