import { useCallback, useEffect, useRef, useState } from 'react';
import { TauriAPI } from './api/tauri-client';
import type { Segment } from './api/tauri-client';
import type { RawSegment } from './api/tauri-client';
import { ControlPanel } from './components/ControlPanel';
import { SegmentCard } from './components/SegmentCard';
import { AudioPlayer } from './components/AudioPlayer';
import { useAppStore } from './store/useAppStore';
import { convertFileSrc } from '@tauri-apps/api/core';
import { Icon } from './components/ui/Icon';
import { Button } from './components/ui/Button';
import { RecordCard } from './components/RecordCard';
import { SettingsModal } from './components/SettingsModal';
import { CorrectionModal } from './components/CorrectionModal';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { LogicalSize } from '@tauri-apps/api/dpi';

const DEFAULT_SRT_PATH = 'D:\\temp\\streamspeech.srt';
const DEFAULT_AUDIO_PATH = 'D:\\temp\\streamspeech.wav';
const SIMPLE_WINDOW_SIZE = { width: 560, height: 280 };
const DETAILED_WINDOW_MIN_SIZE = { width: 900, height: 600 };
const DETAILED_WINDOW_SIZE = { width: 1280, height: 820 };


function formatStamp(seconds: number): string {
  const mins = Math.floor(seconds / 60);
  const secs = Math.floor(seconds % 60);
  const centi = Math.floor((seconds * 100) % 100);
  return `${String(mins).padStart(2, '0')}:${String(secs).padStart(2, '0')}.${String(centi).padStart(2, '0')}`;
}

async function handleWindowDragStart(event: React.MouseEvent<HTMLElement>) {
  if (event.button !== 0) {
    return;
  }

  const target = event.target;
  if (target instanceof HTMLElement) {
    const interactiveSelector = 'button, a, input, textarea, select, [role="button"], [data-no-drag="true"], .no-drag';
    if (target.closest(interactiveSelector)) {
      return;
    }
  }

  try {
    await getCurrentWindow().startDragging();
  } catch (err) {
    console.error('Start dragging window failed', err);
  }
}

function App() {
  const store = useAppStore();
  const pollTimer = useRef<number | null>(null);
  const [activeSegmentId, setActiveSegmentId] = useState<number | null>(null);
  const [seekTo, setSeekTo] = useState<number | null>(null);
  const [isBusy, setIsBusy] = useState(false);
  const [showSettingsModal, setShowSettingsModal] = useState(false);
  const [showCorrectionModal, setShowCorrectionModal] = useState(false);
  const [copiedState, setCopiedState] = useState<'zh' | 'en' | null>(null);
  const copyFeedbackTimerRef = useRef<number | null>(null);

  // Recording Logic
  const stopPolling = useCallback(() => {
    if (pollTimer.current) {
      clearInterval(pollTimer.current);
      pollTimer.current = null;
    }
  }, []);

  const startPolling = useCallback(() => {
    if (pollTimer.current) clearInterval(pollTimer.current);
    pollTimer.current = window.setInterval(async () => {
      try {
        const state = await TauriAPI.getRecordingState();
        store.setElapsedTime(state.elapsed_secs);

        const mappedSegments: Segment[] = state.segments.map((s: RawSegment) => ({
          id: s.segment_id,
          start: s.start,
          end: s.end,
          wall_start: s.wall_start,
          wall_end: s.wall_end,
          text_raw: s.text,
          text_optimized: s.text_optimized,
          text_english: s.text_english,
          opt_status: s.opt_status || 'done',
        }));

        store.setSegments(mappedSegments);

        const hasPending = mappedSegments.some((seg) => seg.opt_status === 'running' || seg.opt_status === 'pending');
        if (state.recording) {
          store.setStatus(hasPending ? 'processing' : 'recording');
          return;
        }

        if (hasPending) {
          store.setStatus('processing');
          return;
        }

        stopPolling();
        store.setStatus('finished');

        try {
          const path = await TauriAPI.getRecordedAudioPath();
          store.setAudioUrl(convertFileSrc(path));
        } catch (err) {
          console.error("Load audio failed", err);
        }
      } catch (err) {
        console.error("Poll failed", err);
        stopPolling();
        store.setStatus('error');
      }
    }, 500);
  }, [store, stopPolling]);

  useEffect(() => () => stopPolling(), [stopPolling]);

  const startRecording = async () => {
    try {
      setIsBusy(true);
      await TauriAPI.startRecording();
      store.setStatus('recording');
      store.setSegments([]);
      store.setElapsedTime(0);
      store.setAudioUrl(null);
      setCopiedState(null);
      startPolling();
    } catch (err) {
      console.error("Start failed", err);
      store.setStatus('error');
    } finally {
      setIsBusy(false);
    }
  };

  const stopRecording = async () => {
    try {
      setIsBusy(true);
      store.setStatus('processing');
      await TauriAPI.stopRecording();
    } catch (err) {
      console.error("Stop failed", err);
      store.setStatus('error');
    } finally {
      setIsBusy(false);
    }
  };
  const handleSeek = (time: number) => {
    setSeekTo(time);
  };

  const setCopyFeedback = (target: 'zh' | 'en') => {
    setCopiedState(target);
    if (copyFeedbackTimerRef.current !== null) {
      window.clearTimeout(copyFeedbackTimerRef.current);
    }
    copyFeedbackTimerRef.current = window.setTimeout(() => {
      setCopiedState(null);
      copyFeedbackTimerRef.current = null;
    }, 1200);
  };

  const handleCopy = async (text: string, target?: 'zh' | 'en') => {
    await TauriAPI.copyToClipboard(text);
    if (target) {
      setCopyFeedback(target);
    }
  };

  const handleClear = async () => {
    await TauriAPI.clearResults();
    store.setSegments([]);
    store.setAudioUrl(null);
    setCopiedState(null);
  };

  const handleDeviceChange = async (device: string) => {
    store.setSelectedDevice(device);
    try {
      await TauriAPI.setInputDevice(device);
    } catch (err) {
      console.error("Set input device failed", err);
      store.setStatus('error');
    }
  };

  const handleCopyZh = () => {
    const text = store.segments.map((seg) => seg.text_optimized || seg.text_raw).filter(Boolean).join('\n');
    if (text) {
      handleCopy(text, 'zh').catch((err) => console.error('Copy zh failed', err));
    }
  };

  const handleCopyEn = () => {
    const text = store.segments.map((seg) => seg.text_english || '').filter(Boolean).join('\n');
    if (text) {
      handleCopy(text, 'en').catch((err) => console.error('Copy en failed', err));
    }
  };

  const handleCopyWithTimestamp = () => {
    const text = store.segments
      .map((seg) => `[${formatStamp(seg.start)} -> ${formatStamp(seg.end)}] ${seg.text_optimized || seg.text_raw}`)
      .join('\n');
    if (text) {
      handleCopy(text).catch((err) => console.error('Copy timestamp text failed', err));
    }
  };

  const handleExportSrt = async () => {
    const path = window.prompt('请输入 SRT 导出路径', DEFAULT_SRT_PATH);
    if (!path) return;
    try {
      await TauriAPI.exportSrt(path);
    } catch (err) {
      console.error("Export SRT failed", err);
      store.setStatus('error');
    }
  };

  const handleSaveAudio = async () => {
    const path = window.prompt('请输入音频导出路径', DEFAULT_AUDIO_PATH);
    if (!path) return;
    try {
      await TauriAPI.saveAllAudio(path);
    } catch (err) {
      console.error("Save audio failed", err);
      store.setStatus('error');
    }
  };

  const isSimpleMode = store.uiMode === 'simple';
  const showActions = store.segments.length > 0;
  const displaySegments = store.segments.slice().reverse();
  
  useEffect(() => {
    return () => {
      if (copyFeedbackTimerRef.current !== null) {
        window.clearTimeout(copyFeedbackTimerRef.current);
      }
    };
  }, []);

  useEffect(() => {
    const applyWindowMode = async () => {
      try {
        const win = getCurrentWindow();
        if (isSimpleMode) {
          await win.setMinSize(null);
          await win.setAlwaysOnTop(true);
          await win.setSize(new LogicalSize(SIMPLE_WINDOW_SIZE.width, SIMPLE_WINDOW_SIZE.height));

          window.setTimeout(() => {
            win.setSize(new LogicalSize(SIMPLE_WINDOW_SIZE.width, SIMPLE_WINDOW_SIZE.height)).catch((err) => {
              console.error('Re-apply simple window size failed', err);
            });
          }, 50);
        } else {
          await win.setAlwaysOnTop(false);
          await win.setMinSize(new LogicalSize(DETAILED_WINDOW_MIN_SIZE.width, DETAILED_WINDOW_MIN_SIZE.height));
          await win.setSize(new LogicalSize(DETAILED_WINDOW_SIZE.width, DETAILED_WINDOW_SIZE.height));
        }
      } catch (err) {
        console.error("Apply window mode failed", err);
      }
    };
    applyWindowMode();
  }, [isSimpleMode]);

  return (
    <div className="flex h-screen overflow-hidden bg-[var(--bg-canvas)] rounded-[18px] border border-[var(--line)]">
      {isSimpleMode && (
        <main className="w-full h-full p-2.5 flex items-center justify-center bg-transparent">
          <section className="w-[520px] max-w-full h-auto rounded-[18px] border border-[var(--line)] bg-[var(--bg-app)] shadow-[0_18px_48px_rgba(15,23,42,0.16)] px-3 py-2.5 flex flex-col gap-2 drag-region" onMouseDown={handleWindowDragStart}>
            <div className="flex items-center justify-between min-h-8 drag-region select-none" onMouseDown={handleWindowDragStart}>
              <div className="flex items-center gap-2">
                <div className="w-6 h-6 rounded-[7px] bg-gradient-to-br from-[var(--primary)] to-[var(--primary-deep)] text-white flex items-center justify-center">
                  <Icon name="logo" size={14} stroke={2} />
                </div>
                <span className="text-[12px] font-semibold text-[var(--ink)]">简洁模式</span>
              </div>
              <div className="flex items-center gap-1">
                <Button variant="ghost" size="icon" className="w-7 h-7 rounded-full" onClick={() => store.setUiMode('detailed')}>
                  <Icon name="search" size={14} className="text-[var(--ink-3)]" />
                </Button>
              </div>
            </div>

            <RecordCard
              status={store.status}
              elapsedTime={store.elapsedTime}
              onStart={startRecording}
              onStop={stopRecording}
              disabled={store.devices.length === 0 || isBusy}
            />
          </section>
        </main>
      )}

      {!isSimpleMode && (
        <div className="flex min-w-0 flex-1 h-full">
          <ControlPanel
            status={store.status}
            elapsedTime={store.elapsedTime}
            devices={store.devices}
            selectedDevice={store.selectedDevice}
            onDeviceChange={handleDeviceChange}
            showEnglish={store.showEnglish}
            onShowEnglishChange={store.setShowEnglish}
            onStart={startRecording}
            onStop={stopRecording}
            onClear={handleClear}
            onShowSettings={() => setShowSettingsModal(true)}
            onShowRules={() => setShowCorrectionModal(true)}
            onToggleMode={() => store.setUiMode(store.uiMode === 'detailed' ? 'simple' : 'detailed')}
            disabled={isBusy}
          />
        </div>
      )}

      {!isSimpleMode && (
        <main className="flex-1 flex flex-col min-w-0">
          <div className="h-[62px] px-6 border-b border-[var(--line)] flex items-center gap-2">
            <Button variant={copiedState === 'zh' ? 'soft' : 'outline'} size="sm" disabled={!showActions} onClick={handleCopyZh}>
              {copiedState === 'zh' ? '中文已复制' : '复制中文'}
            </Button>
            <Button variant={copiedState === 'en' ? 'soft' : 'outline'} size="sm" disabled={!showActions} onClick={handleCopyEn}>
              {copiedState === 'en' ? '英文已复制' : '复制英文'}
            </Button>
            <Button variant="outline" size="sm" disabled={!showActions} onClick={handleCopyWithTimestamp}>含时间戳</Button>
            <Button variant="outline" size="sm" disabled={!showActions} onClick={handleExportSrt}>导出 SRT</Button>
            <Button variant="outline" size="sm" disabled={!showActions} onClick={handleSaveAudio}>保存音频</Button>
            {!showActions && (
              <span className="ml-auto text-[12px] text-[var(--ink-4)]">开始录音后将启用导出</span>
            )}
          </div>
          <div className="flex-1 overflow-y-auto p-6 px-10">
            <div className="max-w-3xl mx-auto flex flex-col gap-4">
              {store.segments.length === 0 && store.status === 'idle' && (
                <div className="flex flex-col items-center justify-center py-40 gap-4 opacity-30">
                  <Icon name="mic" size={48} stroke={1.2} />
                  <p className="text-sm font-medium">准备就绪，点击“开始录音”开始识别</p>
                </div>
              )}

              {displaySegments.map((seg, idx) => (
                <SegmentCard
                  key={idx}
                  segment={seg}
                  isActive={activeSegmentId !== null && seg.id === activeSegmentId}
                  showEnglish={store.showEnglish}
                  onSeek={handleSeek}
                  onCopy={(text) => {
                    handleCopy(text).catch((err) => console.error('Copy segment text failed', err));
                  }}
                />
              ))}
            </div>
          </div>

          <AudioPlayer url={store.audioUrl} seekTo={seekTo} onTimeUpdate={(t) => {
            const active = store.segments.find(s => t >= s.start && t <= s.end);
            setActiveSegmentId(active?.id ?? null);
          }} />
        </main>
      )}
      <SettingsModal open={showSettingsModal} onClose={() => setShowSettingsModal(false)} />
      <CorrectionModal open={showCorrectionModal} onClose={() => setShowCorrectionModal(false)} />
    </div>
  );
}

export default App;
