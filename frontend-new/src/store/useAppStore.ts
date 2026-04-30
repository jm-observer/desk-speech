import { useState, useEffect } from 'react';
import { TauriAPI } from '../api/tauri-client';
import type { Segment } from '../api/tauri-client';

export type AppStatus = 'idle' | 'initializing' | 'recording' | 'processing' | 'error' | 'finished';

export const useAppStore = () => {
  const [status, setStatus] = useState<AppStatus>('initializing');
  const [segments, setSegments] = useState<Segment[]>([]);
  const [devices, setDevices] = useState<{ label: string; value: string }[]>([]);
  const [selectedDevice, setSelectedDevice] = useState<string>('');
  const [elapsedTime, setElapsedTime] = useState(0);
  const [autoCopy, setAutoCopy] = useState(true);
  const [showEnglish, setShowEnglish] = useState(true);
  const [uiMode, setUiMode] = useState<'detailed' | 'simple'>('detailed');
  const [audioUrl, setAudioUrl] = useState<string | null>(null);

  // Initialize
  useEffect(() => {
    let canceled = false;
    let initTimer: number | null = null;

    const init = async () => {
      try {
        const devices = await TauriAPI.listDevices();
        if (canceled) return;
        setDevices(devices.map(d => ({ label: d.is_default ? `${d.name} (Default)` : d.name, value: d.name })));
        
        const selected = await TauriAPI.getSelectedDevice();
        if (canceled) return;
        if (selected) setSelectedDevice(selected);

        // Check init status
        pollInit();
      } catch (err) {
        if (canceled) return;
        console.error("Init failed", err);
        setStatus('error');
      }
    };

    const pollInit = async () => {
      if (canceled) return;
      const res = await TauriAPI.getInitStatus();
      if (canceled) return;
      if (res.status === 1) {
        setStatus('idle');
      } else if (res.status === 2) {
        setStatus('error');
      } else {
        initTimer = window.setTimeout(pollInit, 500);
      }
    };

    init();

    return () => {
      canceled = true;
      if (initTimer !== null) {
        window.clearTimeout(initTimer);
      }
    };
  }, []);

  return {
    status, setStatus,
    segments, setSegments,
    devices, setDevices,
    selectedDevice, setSelectedDevice,
    elapsedTime, setElapsedTime,
    autoCopy, setAutoCopy,
    showEnglish, setShowEnglish,
    uiMode, setUiMode,
    audioUrl, setAudioUrl
  };
};
