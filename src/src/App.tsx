import { useCallback, useEffect, useRef, useState } from 'react';
import { TauriAPI } from './api/tauri-client';
import type { Segment } from './api/tauri-client';
import type { RawSegment } from './api/tauri-client';
import { ControlPanel } from './components/ControlPanel';
import { SegmentCard } from './components/SegmentCard';
import { useAppStore } from './store/useAppStore';
import { Icon } from './components/ui/Icon';
import { RecordCard } from './components/RecordCard';
import { SettingsModal } from './components/SettingsModal';
import { CorrectionModal } from './components/CorrectionModal';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { LogicalSize } from '@tauri-apps/api/dpi';
import { Button } from './components/ui/Button';

const SIMPLE_WINDOW_SIZE = { width: 560, height: 280 };
const DETAILED_WINDOW_MIN_SIZE = { width: 900, height: 600 };
const DETAILED_WINDOW_SIZE = { width: 1280, height: 820 };


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
  const [isBusy, setIsBusy] = useState(false);
  const [showSettingsModal, setShowSettingsModal] = useState(false);
  const [showCorrectionModal, setShowCorrectionModal] = useState(false);

  const mergeSegmentsByRevision = useCallback((incoming: Segment[]) => {
    store.setSegments((prev) => {
      if (prev.length === 0) {
        return incoming;
      }
      // Use segment ID as the primary key, fallback to start time.
      // This ensures that when a segment is updated (revision changes), we update the same record
      // instead of adding a new one.
      const merged = new Map<string, Segment>();
      
      prev.forEach((seg) => {
        const key = seg.id !== null ? `id-${seg.id}` : `start-${seg.start.toFixed(3)}`;
        merged.set(key, seg);
      });
      
      incoming.forEach((seg) => {
        const key = seg.id !== null ? `id-${seg.id}` : `start-${seg.start.toFixed(3)}`;
        const current = merged.get(key);
        // Update if not exists or if incoming has a newer revision
        if (!current || (seg.revision ?? 0) >= (current.revision ?? 0)) {
          merged.set(key, seg);
        }
      });
      
      return Array.from(merged.values()).sort((a, b) => a.start - b.start);
    });
  }, [store]);

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
          revision: s.revision,
          start: s.start,
          end: s.end,
          wall_start: s.wall_start,
          wall_end: s.wall_end,
          text_raw: s.text,
          text_optimized: s.text_optimized,
          text_english: s.text_english,
          optimize_status: s.optimize_status || 'pending',
          translate_status: s.translate_status || 'blocked',
        }));
        mergeSegmentsByRevision(mappedSegments);

        const hasPending = mappedSegments.some(
          (seg) =>
            seg.optimize_status === 'pending' ||
            seg.optimize_status === 'running' ||
            seg.translate_status === 'pending' ||
            seg.translate_status === 'running'
        );
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
      } catch (err) {
        console.error("Poll failed", err);
        stopPolling();
        store.setStatus('error');
      }
    }, 500);
  }, [mergeSegmentsByRevision, store, stopPolling]);

  useEffect(() => () => stopPolling(), [stopPolling]);

  const startRecording = async () => {
    try {
      setIsBusy(true);
      await TauriAPI.startRecording();
      store.setStatus('recording');
      store.setSegments([]);
      store.setElapsedTime(0);
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

  const handleCopy = async (text: string) => {
    await TauriAPI.copyToClipboard(text);
  };

  const handleSegmentCopy = (text: string, _source: 'english' | 'optimized' | 'raw') => {
    handleCopy(text).catch((err) => console.error('Copy segment text failed', err));
  };

  const handleClear = async () => {
    await TauriAPI.clearResults();
    store.setSegments([]);
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

  const isSimpleMode = store.uiMode === 'simple';
  const displaySegments = store.segments.slice().reverse();

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
        <div className="shrink-0 h-full">
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
          <div className="flex-1 overflow-y-auto p-4 px-6">
            <div className="max-w-none mx-0 flex flex-col gap-3">
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
                  showEnglish={store.showEnglish}
                  onCopyChinese={(text) => handleSegmentCopy(text, 'optimized')}
                  onCopyEnglish={(text) => handleSegmentCopy(text, 'english')}
                />
              ))}
            </div>
          </div>
        </main>
      )}
      <SettingsModal open={showSettingsModal} onClose={() => setShowSettingsModal(false)} />
      <CorrectionModal open={showCorrectionModal} onClose={() => setShowCorrectionModal(false)} />
    </div>
  );
}

export default App;
