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
import { getCurrentWindow, UserAttentionType } from '@tauri-apps/api/window';
import { LogicalSize } from '@tauri-apps/api/dpi';
import { isPermissionGranted, requestPermission, sendNotification } from '@tauri-apps/plugin-notification';
import { Button } from './components/ui/Button';

const SIMPLE_WINDOW_SIZE = { width: 560, height: 280 };
const DETAILED_WINDOW_MIN_SIZE = { width: 900, height: 600 };
const DETAILED_WINDOW_SIZE = { width: 1280, height: 820 };
const AUTO_RECORDING_STORAGE_KEY = 'streaming-speech:auto-recording';
const MANUAL_REFRESH_INTERVAL_MS = 800;
const MANUAL_REFRESH_MAX_ATTEMPTS = 25;


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
  const notifiedRevisionsRef = useRef<Set<string>>(new Set());
  const manualTriggeredRevisionsRef = useRef<Set<number>>(new Set());
  const notificationBaselineReadyRef = useRef(false);
  const autoStartTriggeredRef = useRef(false);
  const [isBusy, setIsBusy] = useState(false);
  const [showSettingsModal, setShowSettingsModal] = useState(false);
  const [showCorrectionModal, setShowCorrectionModal] = useState(false);
  const [autoRecordingEnabled, setAutoRecordingEnabled] = useState(() => {
    const saved = window.localStorage.getItem(AUTO_RECORDING_STORAGE_KEY);
    return saved === null ? true : saved === 'true';
  });

  const logNotificationDebug = useCallback((message: string, details?: Record<string, unknown>) => {
    if (details) {
      console.info('[notification-debug]', message, details);
      return;
    }
    console.info('[notification-debug]', message);
  }, []);

  const logWindowDebug = useCallback((message: string, details?: Record<string, unknown>) => {
    if (details) {
      console.info('[window-debug]', message, details);
      return;
    }
    console.info('[window-debug]', message);
  }, []);

  const showTranslationNotification = useCallback(async (segment: Segment) => {
    logNotificationDebug('start notification flow', {
      revision: segment.revision,
      segmentId: segment.segment_id,
      translateStatus: segment.translate_status,
      hasEnglish: Boolean(segment.text_english?.trim()),
    });

    let permissionGranted = await isPermissionGranted();
    logNotificationDebug('checked notification permission', { permissionGranted });

    if (!permissionGranted) {
      const permission = await requestPermission();
      permissionGranted = permission === 'granted';
      logNotificationDebug('requested notification permission', {
        permission,
        permissionGranted,
      });
    }

    if (!permissionGranted) {
      logNotificationDebug('notification aborted: permission not granted', {
        revision: segment.revision,
      });
      return;
    }

    const body = segment.text_english?.trim() || '有新的翻译结果可查看';
    logNotificationDebug('sending native notification', {
      revision: segment.revision,
      bodyPreview: body.slice(0, 60),
      bodyLength: body.length,
    });
    sendNotification({
      title: '识别完成',
      body: body.slice(0, 120),
    });
    
    try {
      logNotificationDebug('requesting window attention (jumping)');
      await getCurrentWindow().requestUserAttention(UserAttentionType.Critical);
    } catch (err) {
      console.error('Request window attention failed', err);
    }

    logNotificationDebug('notification sequence completed', {
      revision: segment.revision,
    });
  }, [logNotificationDebug]);

  useEffect(() => {
    logNotificationDebug('checking notification permission on mount');
    isPermissionGranted().catch((err) => {
      console.error('Check notification permission failed', err);
    });
  }, [logNotificationDebug]);

  const getSegmentKey = useCallback((seg: Segment) => {
    // 优先使用后端分配的稳定 ID
    if (seg.segment_id !== null && seg.segment_id !== undefined) {
      return `seg-${seg.segment_id}`;
    }
    // 其次使用数据库自增 ID
    if (seg.id !== null && seg.id !== undefined) {
      return `db-${seg.id}`;
    }
    // 最后使用开始时间戳（保留3位小数防止浮点误差）
    return `ts-${seg.start.toFixed(3)}`;
  }, []);

  const mergeSegmentsByRevision = useCallback((incoming: Segment[]) => {
    store.setSegments((prev) => {
      if (prev.length === 0) {
        return incoming;
      }
      const merged = new Map<string, Segment>();

      prev.forEach((seg) => {
        merged.set(getSegmentKey(seg), seg);
      });

      incoming.forEach((seg) => {
        const key = getSegmentKey(seg);
        const current = merged.get(key);
        if (!current || (seg.revision ?? 0) >= (current.revision ?? 0)) {
          merged.set(key, seg);
        }
      });
      
      return Array.from(merged.values()).sort((a, b) => {
        if (a.wall_start !== b.wall_start) {
          return a.wall_start.localeCompare(b.wall_start);
        }
        return a.start - b.start;
      });
    });
  }, [getSegmentKey, store]);

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

        const mappedSegments: Segment[] = state.segments.map((s: RawSegment) => ({
          id: s.segment_id,
          segment_id: s.segment_id,
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
        console.debug('[segments][memory-poll]', {
          recording: state.recording,
          segmentCount: mappedSegments.length,
          firstSegmentId: mappedSegments[0]?.segment_id ?? null,
          lastSegmentId: mappedSegments[mappedSegments.length - 1]?.segment_id ?? null,
          firstRevision: mappedSegments[0]?.revision ?? null,
          lastRevision: mappedSegments[mappedSegments.length - 1]?.revision ?? null,
        });
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
    }, 1000);
  }, [mergeSegmentsByRevision, store, stopPolling]);

  useEffect(() => () => stopPolling(), [stopPolling]);

  useEffect(() => {
    const currentWindow = getCurrentWindow();
    let unlisten: (() => void) | undefined;

    const bindMinimizeToTray = async () => {
      logWindowDebug('binding minimize listener');
      unlisten = await currentWindow.onResized(async () => {
        try {
          const minimized = await currentWindow.isMinimized();
          logWindowDebug('window resized event received', { minimized });
          if (minimized) {
            logWindowDebug('window minimized, hiding to tray');
            await currentWindow.hide();
            logWindowDebug('window hidden after minimize');
          }
        } catch (err) {
          console.error('Hide minimized window failed', err);
        }
      });
    };

    bindMinimizeToTray().catch((err) => {
      console.error('Bind minimize listener failed', err);
    });

    return () => {
      if (unlisten) {
        unlisten();
      }
    };
  }, []);

  useEffect(() => {
    if (!notificationBaselineReadyRef.current) {
      store.segments.forEach((seg) => {
        if (seg.revision !== undefined) {
          const optKey = `opt-${seg.revision}`;
          const transKey = `trans-${seg.revision}`;
          if (seg.optimize_status === 'success') notifiedRevisionsRef.current.add(optKey);
          if (seg.translate_status === 'success') notifiedRevisionsRef.current.add(transKey);
        }
      });
      notificationBaselineReadyRef.current = true;
      logNotificationDebug('notification baseline initialized', {
        initialTrackedCount: notifiedRevisionsRef.current.size,
      });
      return;
    }

    store.segments.forEach((segment) => {
      const revision = segment.revision;
      if (revision === undefined) return;

      if (manualTriggeredRevisionsRef.current.has(revision)) {
        if (
          segment.optimize_status === 'failed' ||
          segment.translate_status === 'failed' ||
          segment.translate_status === 'success'
        ) {
          manualTriggeredRevisionsRef.current.delete(revision);
        }
        return;
      }

      // Check optimization status
      const optKey = `opt-${revision}`;
      if (segment.optimize_status === 'success' && !notifiedRevisionsRef.current.has(optKey)) {
        logNotificationDebug('triggering notification: optimization success', { revision });
        notifiedRevisionsRef.current.add(optKey);
        showTranslationNotification(segment).catch(console.error);
      }

      // Check translation status
      const transKey = `trans-${revision}`;
      if (segment.translate_status === 'success' && !notifiedRevisionsRef.current.has(transKey)) {
        logNotificationDebug('triggering notification: translation success', { revision });
        notifiedRevisionsRef.current.add(transKey);
        showTranslationNotification(segment).catch(console.error);
      }
    });
  }, [logNotificationDebug, showTranslationNotification, store.segments]);

  // Sync initial recording state on mount
  useEffect(() => {
    if (!store.isInitialized) return;
    
    const sync = async () => {
      try {
        const state = await TauriAPI.getRecordingState();
        if (state.recording) {
          startPolling();
        }
      } catch (err) {
        console.error("Initial sync recording state failed", err);
      }
    };
    sync();
  }, [store.isInitialized, startPolling]);

  const startRecording = async () => {
    try {
      setIsBusy(true);
      await TauriAPI.startRecording();
      store.setStatus('recording');
      startPolling();
    } catch (err) {
      console.error("Start failed", err);
      store.setStatus('error');
    } finally {
      setIsBusy(false);
    }
  };
  const isSimpleMode = store.uiMode === 'simple';

  useEffect(() => {
    if (!store.isInitialized) {
      return;
    }
    if (autoStartTriggeredRef.current) {
      return;
    }
    // 只有在空闲或刚结束，且确认后端没有正在录音时才尝试自动启动
    if (store.status !== 'idle' && store.status !== 'finished') {
      return;
    }
    if (store.devices.length === 0) {
      return;
    }
    if (!autoRecordingEnabled) {
      return;
    }

    autoStartTriggeredRef.current = true;
    startRecording().catch((err) => {
      console.error('Auto start recording failed', err);
    });
  }, [autoRecordingEnabled, startRecording, store.devices.length, store.status, store.isInitialized]);

  useEffect(() => {
    window.localStorage.setItem(AUTO_RECORDING_STORAGE_KEY, String(autoRecordingEnabled));
  }, [autoRecordingEnabled]);

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

  const handleManualOptimizeTranslate = useCallback(
    async (segment: Segment) => {
      if (segment.revision === undefined) {
        return;
      }

      try {
        const targetRevision = segment.revision;
        manualTriggeredRevisionsRef.current.add(segment.revision);
        await TauriAPI.manualOptimizeTranslate(targetRevision);
        if (store.status !== 'recording' && store.status !== 'processing') {
          store.setStatus('processing');
        }
        let attempts = 0;
        while (attempts < MANUAL_REFRESH_MAX_ATTEMPTS) {
          await new Promise((resolve) => window.setTimeout(resolve, MANUAL_REFRESH_INTERVAL_MS));
          attempts += 1;

          const rows = await TauriAPI.listSegments(0, 200);
          const mapped = rows
            .map((row) => ({
              id: typeof row.id === 'number' ? row.id : null,
              segment_id: typeof row.segment_id === 'number' ? row.segment_id : null,
              revision: typeof row.revision === 'number' ? row.revision : undefined,
              start: typeof row.start_sec === 'number' ? row.start_sec : 0,
              end: typeof row.end_sec === 'number' ? row.end_sec : 0,
              wall_start: typeof row.wall_start === 'string' ? row.wall_start : '',
              wall_end: typeof row.wall_end === 'string' ? row.wall_end : '',
              text_raw: typeof row.text_raw === 'string' ? row.text_raw : '',
              text_optimized: typeof row.text_optimized === 'string' ? row.text_optimized : undefined,
              text_english: typeof row.text_english === 'string' ? row.text_english : undefined,
              optimize_status: (row.optimize_status === 'pending' ||
              row.optimize_status === 'running' ||
              row.optimize_status === 'success' ||
              row.optimize_status === 'failed'
                ? row.optimize_status
                : 'pending') as Segment['optimize_status'],
              translate_status: (row.translate_status === 'blocked' ||
              row.translate_status === 'pending' ||
              row.translate_status === 'running' ||
              row.translate_status === 'success' ||
              row.translate_status === 'failed'
                ? row.translate_status
                : 'blocked') as Segment['translate_status'],
            }))
            .filter((seg) => seg.text_raw.trim().length > 0);
          mergeSegmentsByRevision(mapped);

          const target = mapped.find((item) => item.revision === targetRevision);
          if (!target) {
            continue;
          }
          const optimizeDone = target.optimize_status === 'success' || target.optimize_status === 'failed';
          const translateDone = target.translate_status === 'success' || target.translate_status === 'failed' || target.translate_status === 'blocked';
          if (optimizeDone && translateDone) {
            break;
          }
        }
      } catch (err) {
        manualTriggeredRevisionsRef.current.delete(segment.revision);
        console.error('Manual optimize translate failed', err);
        alert(`手动优化与翻译失败: ${err}`);
      }
    },
    [mergeSegmentsByRevision, store]
  );

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
            devices={store.devices}
            selectedDevice={store.selectedDevice}
            onDeviceChange={handleDeviceChange}
            showEnglish={store.showEnglish}
            onShowEnglishChange={store.setShowEnglish}
            autoRecordingEnabled={autoRecordingEnabled}
            onAutoRecordingEnabledChange={setAutoRecordingEnabled}
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

              {displaySegments.map((seg) => (
                <SegmentCard
                  key={getSegmentKey(seg)}
                  segment={seg}
                  showEnglish={store.showEnglish}
                  onCopyChinese={(text) => handleSegmentCopy(text, 'optimized')}
                  onCopyEnglish={(text) => handleSegmentCopy(text, 'english')}
                  onManualOptimizeTranslate={handleManualOptimizeTranslate}
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
