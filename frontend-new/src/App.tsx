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
import { SettingsModal } from './components/SettingsModal';
import { CorrectionModal } from './components/CorrectionModal';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { LogicalSize } from '@tauri-apps/api/dpi';

const DEFAULT_SRT_PATH = 'D:\\temp\\streamspeech.srt';
const DEFAULT_AUDIO_PATH = 'D:\\temp\\streamspeech.wav';

function formatStamp(seconds: number): string {
  const mins = Math.floor(seconds / 60);
  const secs = Math.floor(seconds % 60);
  const centi = Math.floor((seconds * 100) % 100);
  return `${String(mins).padStart(2, '0')}:${String(secs).padStart(2, '0')}.${String(centi).padStart(2, '0')}`;
}

function App() {
  const store = useAppStore();
  const pollTimer = useRef<number | null>(null);
  const [activeSegmentId, setActiveSegmentId] = useState<number | null>(null);
  const [seekTo, setSeekTo] = useState<number | null>(null);
  const [isBusy, setIsBusy] = useState(false);
  const [showSettingsModal, setShowSettingsModal] = useState(false);
  const [showCorrectionModal, setShowCorrectionModal] = useState(false);

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

  const handleCopy = (text: string) => {
    TauriAPI.copyToClipboard(text);
  };

  const handleClear = async () => {
    await TauriAPI.clearResults();
    store.setSegments([]);
    store.setAudioUrl(null);
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
    if (text) handleCopy(text);
  };

  const handleCopyEn = () => {
    const text = store.segments.map((seg) => seg.text_english || '').filter(Boolean).join('\n');
    if (text) handleCopy(text);
  };

  const handleCopyWithTimestamp = () => {
    const text = store.segments
      .map((seg) => `[${formatStamp(seg.start)} -> ${formatStamp(seg.end)}] ${seg.text_optimized || seg.text_raw}`)
      .join('\n');
    if (text) handleCopy(text);
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

  useEffect(() => {
    const applyWindowMode = async () => {
      try {
        const win = getCurrentWindow();
        if (isSimpleMode) {
          await win.setAlwaysOnTop(true);
          await win.setSize(new LogicalSize(352, 220));
        } else {
          await win.setAlwaysOnTop(false);
          await win.setSize(new LogicalSize(1280, 820));
        }
      } catch (err) {
        console.error("Apply window mode failed", err);
      }
    };
    applyWindowMode();
  }, [isSimpleMode]);

  return (
    <div className="flex h-screen overflow-hidden bg-[var(--bg-canvas)]">
      <ControlPanel
        status={store.status}
        elapsedTime={store.elapsedTime}
        devices={store.devices}
        selectedDevice={store.selectedDevice}
        onDeviceChange={handleDeviceChange}
        autoCopy={store.autoCopy}
        onAutoCopyChange={store.setAutoCopy}
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

      {!isSimpleMode && (
        <main className="flex-1 flex flex-col min-w-0">
          <div className="h-[62px] px-6 border-b border-[var(--line)] flex items-center gap-2">
            <Button variant="outline" size="sm" disabled={!showActions} onClick={handleCopyZh}>复制中文</Button>
            <Button variant="outline" size="sm" disabled={!showActions} onClick={handleCopyEn}>复制英文</Button>
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

              {store.segments.map((seg, idx) => (
                <SegmentCard
                  key={idx}
                  segment={seg}
                  isActive={activeSegmentId !== null && seg.id === activeSegmentId}
                  showEnglish={store.showEnglish}
                  onSeek={handleSeek}
                  onCopy={handleCopy}
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
