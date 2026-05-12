import React, { useEffect, useRef, useState, useCallback } from 'react';
import { cn, stripYear } from '../utils';
import type { Segment } from '../api/tauri-client';
import { Button } from './ui/Button';
import { Icon } from './ui/Icon';

interface SegmentCardProps {
  segment: Segment;
  showEnglish?: boolean;
  onCopyChinese: (text: string) => void;
  onCopyEnglish: (text: string) => void;
  onManualOptimizeTranslate: (segment: Segment) => void;
  onDelete?: (segment: Segment) => void;
}

interface ConfirmDialogProps {
  title: string;
  message: string;
  onConfirm: () => void;
  onCancel: () => void;
}

const ConfirmDialog: React.FC<ConfirmDialogProps> = ({ title, message, onConfirm, onCancel }) => {
  const overlayRef = useRef<HTMLDivElement>(null);

  const handleOverlayClick = useCallback((e: React.MouseEvent<HTMLDivElement>) => {
    if (e.target === e.currentTarget) {
      onCancel();
    }
  }, [onCancel]);

  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.key === 'Escape') {
      onCancel();
    }
  }, [onCancel]);

  useEffect(() => {
    overlayRef.current?.focus();
  }, []);

  return (
    <div
      ref={overlayRef}
      role="dialog"
      aria-modal="true"
      aria-labelledby="confirm-title"
      tabIndex={-1}
      className="fixed inset-0 z-[100] flex items-center justify-center bg-black/40 animate-fade-in"
      onClick={handleOverlayClick}
      onKeyDown={handleKeyDown}
    >
      <div className="bg-[var(--bg-app)] border border-[var(--line)] rounded-2xl shadow-[0_20px_60px_rgba(0,0,0,0.3)] w-[360px] p-6 animate-scale-in">
        <h3 id="confirm-title" className="text-[15px] font-semibold text-[var(--ink)] mb-2">
          {title}
        </h3>
        <p className="text-[13px] text-[var(--ink-3)] leading-relaxed mb-5">
          {message}
        </p>
        <div className="flex gap-2.5 justify-end">
          <Button
            variant="ghost"
            size="sm"
            onClick={onCancel}
            className="h-8 px-3.5 text-[13px]"
          >
            取消
          </Button>
          <Button
            variant="primary"
            size="sm"
            onClick={onConfirm}
            className="h-8 px-3.5 text-[13px] bg-red-500 hover:bg-red-600 text-white shadow-none"
          >
            删除
          </Button>
        </div>
      </div>
    </div>
  );
};

export const SegmentCard: React.FC<SegmentCardProps> = ({
  segment,
  showEnglish,
  onCopyChinese,
  onCopyEnglish,
  onManualOptimizeTranslate,
  onDelete,
}) => {
  const [copiedZh, setCopiedZh] = useState(false);
  const [copiedEn, setCopiedEn] = useState(false);
  const [showConfirm, setShowConfirm] = useState(false);
  const [isDeleting, setIsDeleting] = useState(false);
  const cardRef = useRef<HTMLDivElement | null>(null);
  const maxHeightRef = useRef(0);
  const [minHeight, setMinHeight] = useState<number | undefined>(undefined);

  const handleCopyZh = () => {
    onCopyChinese(segment.text_optimized || segment.text_raw);
    setCopiedZh(true);
    setTimeout(() => setCopiedZh(false), 2000);
  };

  const handleCopyEn = () => {
    onCopyEnglish(segment.text_english || '');
    setCopiedEn(true);
    setTimeout(() => setCopiedEn(false), 2000);
  };

  const handleDeleteClick = () => {
    setShowConfirm(true);
  };

  const handleConfirmDelete = async () => {
    if (!onDelete) return;
    setIsDeleting(true);
    setShowConfirm(false);
    try {
      await onDelete(segment);
    } catch {
      setIsDeleting(false);
    }
  };

  const handleCancelDelete = () => {
    setShowConfirm(false);
  };

  const optimizeRunning = segment.optimize_status === 'running' || segment.optimize_status === 'pending';
  const translateRunning = segment.translate_status === 'running' || segment.translate_status === 'pending';
  const isProcessing = optimizeRunning || translateRunning;
  const duration = segment.end - segment.start;
  const canManualRun = segment.revision !== undefined && segment.text_raw.trim().length > 0 && !isProcessing;

  useEffect(() => {
    const element = cardRef.current;
    if (!element) {
      return;
    }

    // 初始高度同步
    const initialHeight = element.offsetHeight;
    if (initialHeight > maxHeightRef.current) {
      maxHeightRef.current = initialHeight;
      setMinHeight(initialHeight);
    }
    
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        // 使用 borderBoxSize 获取包含 padding 和 border 的实际高度
        const height = entry.borderBoxSize?.[0]?.blockSize ?? entry.contentRect.height;
        if (height > maxHeightRef.current) {
          maxHeightRef.current = height;
          setMinHeight(height);
        }
      }
    });
    observer.observe(element);

    return () => {
      observer.disconnect();
    };
  }, []);

  return (
    <>
      <div
        ref={cardRef}
        style={minHeight !== undefined ? { minHeight: `${minHeight}px` } : undefined}
        className={cn(
          'group relative flex flex-col p-4 px-4.5 gap-2.5 bg-[var(--bg-card)] border border-[var(--line)] rounded-[16px] shadow-[var(--shadow-sm)] transition-shadow transition-colors animate-fade-up',
          'hover:shadow-[var(--shadow-md)] hover:border-[var(--line-strong)]'
        )}
      >
        <div className="flex items-center gap-3">
          <span className="px-2 py-0.5 rounded-md bg-[var(--bg-soft)] font-mono text-[11px] text-[var(--ink-2)]">
            {stripYear(segment.wall_start)} → {stripYear(segment.wall_end)}
          </span>
          <span className="text-[11px] text-[var(--ink-4)]">{duration.toFixed(1)}s</span>

          <div className="flex-1" />

          <div className="flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
            {onDelete && (
              <Button
                variant="ghost"
                size="sm"
                className="h-7 px-2 text-[11px] gap-1.5 text-red-400 hover:text-red-500 hover:bg-red-50"
                disabled={isDeleting}
                onClick={handleDeleteClick}
                title="删除此条记录"
              >
                <Icon name="trash" size={12} />
                {isDeleting ? '删除中...' : '删除'}
              </Button>
            )}
            <Button
              variant="ghost"
              size="sm"
              className="h-7 px-2 text-[11px] gap-1.5"
              disabled={!canManualRun}
              onClick={() => onManualOptimizeTranslate(segment)}
              title="手动优化与翻译"
            >
              <Icon name={isProcessing ? 'refresh' : 'sparkles'} size={12} className={cn(isProcessing && 'animate-spin')} />
              {isProcessing ? '处理中...' : '手动优化与翻译'}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              className={cn('h-7 px-2 text-[11px] gap-1.5 transition-colors', copiedZh && 'text-green-600 bg-green-50')}
              disabled={!segment.text_optimized && !segment.text_raw}
              onClick={handleCopyZh}
              title="复制中文"
            >
              <Icon name={copiedZh ? 'check' : 'copy'} size={12} />
              {copiedZh ? '已复制' : '复制中文'}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              className={cn('h-7 px-2 text-[11px] gap-1.5 transition-colors', copiedEn && 'text-green-600 bg-green-50')}
              disabled={!segment.text_english}
              onClick={handleCopyEn}
              title="复制英文"
            >
              <Icon name={copiedEn ? 'check' : 'languages'} size={12} />
              {copiedEn ? '已复制' : '复制英文'}
            </Button>
            <Button variant="ghost" size="icon" className="w-7 h-7">
              <Icon name="download" size={14} />
            </Button>
          </div>
        </div>

        <div className="flex flex-col gap-1.5">
          <p className="text-[13px] leading-[1.7] text-[var(--ink-2)] break-words text-pretty">{segment.text_raw}</p>

          <p className={cn('text-[15px] leading-[1.7] break-words text-pretty', optimizeRunning && 'text-[var(--ink-4)]')}>
            {segment.optimize_status === 'failed'
              ? '优化失败'
              : segment.text_optimized || (optimizeRunning ? '优化中...' : segment.text_raw)}
          </p>

          {showEnglish && (
            <p className={cn('text-[14px] leading-[1.7] break-words text-pretty', translateRunning && 'text-[var(--ink-4)]')}>
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

      {showConfirm && onDelete && (
        <ConfirmDialog
          title="删除识别记录"
          message="确定要删除这条识别记录吗？此操作不可恢复。"
          onConfirm={handleConfirmDelete}
          onCancel={handleCancelDelete}
        />
      )}
    </>
  );
};
