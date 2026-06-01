import { useCallback, useEffect, useRef, useState } from 'react';
import { TauriAPI, DEFAULT_REMOTE_URL } from './api/tauri-client';
import type { AppSettings, AsrLanguage, AutoCopyMode, Segment } from './api/tauri-client';
import { ControlPanel } from './components/ControlPanel';
import { SegmentCard } from './components/SegmentCard';
import { useAppStore } from './store/useAppStore';
import { Icon } from './components/ui/Icon';
import { RecordCard } from './components/RecordCard';
import { getCurrentWindow, UserAttentionType } from '@tauri-apps/api/window';
import { LogicalSize } from '@tauri-apps/api/dpi';
import { isPermissionGranted, requestPermission, sendNotification } from '@tauri-apps/plugin-notification';
import { Button } from './components/ui/Button';

const SIMPLE_WINDOW_SIZE = { width: 560, height: 280 };
const DETAILED_WINDOW_MIN_SIZE = { width: 900, height: 600 };
const DETAILED_WINDOW_SIZE = { width: 1280, height: 820 };
const AUTO_RECORDING_STORAGE_KEY = 'streaming-speech:auto-recording';


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
  const pollInFlightRef = useRef(false);
  const notifiedRevisionsRef = useRef<Set<string>>(new Set());
  const notificationBaselineReadyRef = useRef(false);
  const autoStartTriggeredRef = useRef(false);
  const [isBusy, setIsBusy] = useState(false);
  const [asrLanguage, setAsrLanguage] = useState<AsrLanguage>('zh');
  const [autoCopyMode, setAutoCopyMode] = useState<AutoCopyMode>('english');
  const [mergeWindowMs, setMergeWindowMs] = useState(3000);
  const [remoteUrl, setRemoteUrl] = useState<string>(DEFAULT_REMOTE_URL);
  const [remoteUrlPresets, setRemoteUrlPresets] = useState<string[]>([]);
  const [wantSecondary, setWantSecondary] = useState(false);
  const [notifySound, setNotifySound] = useState(true);
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

  // Load persisted settings from the local DB on mount.
  useEffect(() => {
    TauriAPI.getSettings()
      .then((s) => {
        setAsrLanguage(s.asr_language);
        setAutoCopyMode(s.auto_copy_mode);
        setMergeWindowMs(s.merge_window_ms);
        setRemoteUrl(s.remote_url || DEFAULT_REMOTE_URL);
        setRemoteUrlPresets(s.remote_url_presets || []);
        setWantSecondary(!!s.want_secondary);
        setNotifySound(s.notify_sound !== false);
      })
      .catch((err) => console.warn('Load settings failed', err));
  }, []);

  const getSegmentKey = useCallback((seg: Segment) => {
    if (seg.segment_id !== null && seg.segment_id !== undefined) {
      return `seg-${seg.segment_id}`;
    }
    if (seg.id !== null && seg.id !== undefined) {
      return `db-${seg.id}`;
    }
    return `ts-${seg.start.toFixed(3)}`;
  }, []);

  const segmentsRef = useRef(store.segments);
  useEffect(() => {
    segmentsRef.current = store.segments;
  }, [store.segments]);

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
      if (pollInFlightRef.current) return;
      pollInFlightRef.current = true;
      try {
        // Surface async connection failures (run_remote_session sets
        // init_status=2 + a message after giving up reconnecting).
        const init = await TauriAPI.getInitStatus();
        if (init.status === 2) {
          store.setErrorMessage(init.error || '无法连接识别服务');
          store.setStatus('error');
          stopPolling();
          return;
        }

        const state = await TauriAPI.getRecordingState();
        const hasPending = segmentsRef.current.some(
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
      } finally {
        pollInFlightRef.current = false;
      }
    }, 1000);
  }, [store, stopPolling]);

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
      store.setErrorMessage('');
      await TauriAPI.startRecording();
      store.setStatus('recording');
      startPolling();
    } catch (err) {
      console.error("Start failed", err);
      store.setErrorMessage(typeof err === 'string' ? err : (err as Error)?.message || String(err));
      store.setStatus('error');
    } finally {
      setIsBusy(false);
    }
  };

  const retryRecording = () => {
    autoStartTriggeredRef.current = true; // a manual retry shouldn't re-arm auto-start
    store.setErrorMessage('');
    store.setStatus('idle');
    startRecording();
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

  // Wait until the backend reports recording=false. Used by the live
  // URL-change reconnect path so we don't start a new session while the
  // previous WebSocket is still tearing down.
  const waitUntilNotRecording = useCallback(async (timeoutMs = 4000): Promise<boolean> => {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      try {
        const s = await TauriAPI.getRecordingState();
        if (!s.recording) return true;
      } catch (err) {
        console.warn('waitUntilNotRecording probe failed', err);
      }
      await new Promise((r) => setTimeout(r, 150));
    }
    return false;
  }, []);

  /**
   * Persist the connection-related slice of settings and, if the URL changed
   * while a session is in flight, transparently stop + restart so the new
   * URL takes effect immediately. Other settings (language / auto-copy /
   * merge window) are picked up live by the backend and don't need a restart.
   */
  const persistAndMaybeReconnect = useCallback(
    async (next: AppSettings, urlChanged: boolean) => {
      try {
        await TauriAPI.applySettings(next);
      } catch (err) {
        console.error('apply settings failed', err);
        return;
      }
      if (!urlChanged) return;
      let wasRecording = false;
      try {
        wasRecording = (await TauriAPI.getRecordingState()).recording;
      } catch (err) {
        console.warn('probe recording state failed', err);
      }
      if (!wasRecording) return;
      try {
        await TauriAPI.stopRecording();
      } catch (err) {
        console.error('stop for reconnect failed', err);
      }
      const settled = await waitUntilNotRecording();
      if (!settled) {
        console.warn('reconnect: previous session did not stop in time');
      }
      // Use the same path manual start does so error reporting stays consistent.
      await startRecording();
    },
    [waitUntilNotRecording, startRecording]
  );

  const handleRemoteUrlSelect = (url: string) => {
    if (url === remoteUrl) return;
    setRemoteUrl(url);
    const next: AppSettings = {
      asr_language: asrLanguage,
      auto_copy_mode: autoCopyMode,
      merge_window_ms: mergeWindowMs,
      remote_url: url,
      remote_url_presets: remoteUrlPresets,
      want_secondary: wantSecondary,
      notify_sound: notifySound,
    };
    persistAndMaybeReconnect(next, true);
  };

  const handleRemoteUrlAdd = (url: string) => {
    const trimmed = url.trim();
    if (!trimmed) return;
    const presetsNext = remoteUrlPresets.includes(trimmed)
      ? remoteUrlPresets
      : [...remoteUrlPresets, trimmed];
    const urlChanged = trimmed !== remoteUrl;
    setRemoteUrlPresets(presetsNext);
    setRemoteUrl(trimmed);
    const next: AppSettings = {
      asr_language: asrLanguage,
      auto_copy_mode: autoCopyMode,
      merge_window_ms: mergeWindowMs,
      remote_url: trimmed,
      remote_url_presets: presetsNext,
      want_secondary: wantSecondary,
      notify_sound: notifySound,
    };
    persistAndMaybeReconnect(next, urlChanged);
  };

  const handleNotifySoundChange = (val: boolean) => {
    if (val === notifySound) return;
    setNotifySound(val);
    const next: AppSettings = {
      asr_language: asrLanguage,
      auto_copy_mode: autoCopyMode,
      merge_window_ms: mergeWindowMs,
      remote_url: remoteUrl,
      remote_url_presets: remoteUrlPresets,
      want_secondary: wantSecondary,
      notify_sound: val,
    };
    // Sound flag is read live by run_remote_session each time a segment
    // completes — no reconnect needed.
    TauriAPI.applySettings(next).catch((err) =>
      console.error('apply notify_sound failed', err)
    );
  };

  const handleWantSecondaryChange = (val: boolean) => {
    if (val === wantSecondary) return;
    setWantSecondary(val);
    const next: AppSettings = {
      asr_language: asrLanguage,
      auto_copy_mode: autoCopyMode,
      merge_window_ms: mergeWindowMs,
      remote_url: remoteUrl,
      remote_url_presets: remoteUrlPresets,
      want_secondary: val,
      notify_sound: notifySound,
    };
    // hello.want_secondary is sent once at session start, so toggling needs
    // a reconnect for the new value to take effect. Same pattern as URL.
    persistAndMaybeReconnect(next, true);
  };

  const handleRemoteUrlRemove = (url: string) => {
    const presetsNext = remoteUrlPresets.filter((p) => p !== url);
    // If the removed preset was selected, fall back to the built-in default.
    const fallback = remoteUrl === url ? DEFAULT_REMOTE_URL : remoteUrl;
    const urlChanged = fallback !== remoteUrl;
    setRemoteUrlPresets(presetsNext);
    setRemoteUrl(fallback);
    const next: AppSettings = {
      asr_language: asrLanguage,
      auto_copy_mode: autoCopyMode,
      merge_window_ms: mergeWindowMs,
      remote_url: fallback,
      remote_url_presets: presetsNext,
      want_secondary: wantSecondary,
      notify_sound: notifySound,
    };
    persistAndMaybeReconnect(next, urlChanged);
  };

  const handleCopy = async (text: string) => {
    await TauriAPI.copyToClipboard(text);
  };

  const handleSegmentCopy = (text: string) => {
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
              onRetry={retryRecording}
              errorMessage={store.errorMessage}
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
            onRetry={retryRecording}
            errorMessage={store.errorMessage}
            onClear={handleClear}
            asrLanguage={asrLanguage}
            onAsrLanguageChange={(v) => {
              setAsrLanguage(v);
              const next: AppSettings = {
                asr_language: v,
                auto_copy_mode: autoCopyMode,
                merge_window_ms: mergeWindowMs,
                remote_url: remoteUrl,
                remote_url_presets: remoteUrlPresets,
                want_secondary: wantSecondary,
                notify_sound: notifySound,
              };
              TauriAPI.applySettings(next).catch((err) => console.error('apply asr_language failed', err));
            }}
            autoCopyMode={autoCopyMode}
            onAutoCopyModeChange={(v) => {
              setAutoCopyMode(v);
              const next: AppSettings = {
                asr_language: asrLanguage,
                auto_copy_mode: v,
                merge_window_ms: mergeWindowMs,
                remote_url: remoteUrl,
                remote_url_presets: remoteUrlPresets,
                want_secondary: wantSecondary,
                notify_sound: notifySound,
              };
              TauriAPI.applySettings(next).catch((err) => console.error('apply auto_copy_mode failed', err));
            }}
            mergeWindowMs={mergeWindowMs}
            onMergeWindowMsChange={(v) => {
              setMergeWindowMs(v);
              const next: AppSettings = {
                asr_language: asrLanguage,
                auto_copy_mode: autoCopyMode,
                merge_window_ms: v,
                remote_url: remoteUrl,
                remote_url_presets: remoteUrlPresets,
                want_secondary: wantSecondary,
                notify_sound: notifySound,
              };
              TauriAPI.applySettings(next).catch((err) => console.error('apply merge_window_ms failed', err));
            }}
            remoteUrl={remoteUrl}
            remoteUrlPresets={remoteUrlPresets}
            onRemoteUrlSelect={handleRemoteUrlSelect}
            onRemoteUrlAdd={handleRemoteUrlAdd}
            onRemoteUrlRemove={handleRemoteUrlRemove}
            wantSecondary={wantSecondary}
            onWantSecondaryChange={handleWantSecondaryChange}
            notifySound={notifySound}
            onNotifySoundChange={handleNotifySoundChange}
            onToggleMode={() => store.setUiMode(store.uiMode === 'detailed' ? 'simple' : 'detailed')}
            disabled={isBusy}
          />
        </div>
      )}

      {!isSimpleMode && (
        <main className="flex-1 flex flex-col min-w-0">
          <div className="flex-1 overflow-y-auto p-4 px-6">
            <div className="max-w-none mx-0 flex flex-col gap-3">
              {store.segments.length === 0 && (store.status === 'idle' || store.status === 'finished') && (
                <div className="flex flex-col items-center justify-center py-40 gap-4 opacity-30">
                  <Icon name="mic" size={48} stroke={1.2} />
                  <p className="text-sm font-medium">
                    {store.status === 'idle' ? '准备就绪，点击“开始录音”开始识别' : '当前没有可展示的识别结果'}
                  </p>
                </div>
              )}

              {displaySegments.map((seg) => (
                <SegmentCard
                  key={getSegmentKey(seg)}
                  segment={seg}
                  showEnglish={store.showEnglish}
                  onCopyChinese={(text) => handleSegmentCopy(text)}
                  onCopyEnglish={(text) => handleSegmentCopy(text)}
                />
              ))}
            </div>
          </div>
        </main>
      )}
    </div>
  );
}

export default App;
