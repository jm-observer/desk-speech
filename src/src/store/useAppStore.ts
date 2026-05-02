import { useState, useEffect, useCallback } from 'react';
import { TauriAPI } from '../api/tauri-client';
import type { Segment } from '../api/tauri-client';

export type AppStatus = 'idle' | 'initializing' | 'recording' | 'processing' | 'error' | 'finished';

export const useAppStore = () => {
  const [status, setStatus] = useState<AppStatus>('initializing');
  const [segments, setSegments] = useState<Segment[]>([]);
  const [devices, setDevices] = useState<{ label: string; value: string }[]>([]);
  const [selectedDevice, setSelectedDevice] = useState<string>('');
  const [elapsedTime, setElapsedTime] = useState(0);
  const [showEnglish, setShowEnglish] = useState(true);
  const [uiMode, setUiMode] = useState<'detailed' | 'simple'>('detailed');

  const mapDbSegment = useCallback((row: Record<string, unknown>): Segment => ({
    id: typeof row.id === 'number' ? row.id : null,
    revision: typeof row.revision === 'number' ? row.revision : undefined,
    start: typeof row.start_sec === 'number' ? row.start_sec : 0,
    end: typeof row.end_sec === 'number' ? row.end_sec : 0,
    wall_start: typeof row.wall_start === 'string' ? row.wall_start : '',
    wall_end: typeof row.wall_end === 'string' ? row.wall_end : '',
    text_raw: typeof row.text_raw === 'string' ? row.text_raw : '',
    text_optimized: typeof row.text_optimized === 'string' ? row.text_optimized : undefined,
    text_english: typeof row.text_english === 'string' ? row.text_english : undefined,
    optimize_status:
      row.optimize_status === 'pending' ||
      row.optimize_status === 'running' ||
      row.optimize_status === 'success' ||
      row.optimize_status === 'failed'
        ? row.optimize_status
        : 'pending',
    translate_status:
      row.translate_status === 'blocked' ||
      row.translate_status === 'pending' ||
      row.translate_status === 'running' ||
      row.translate_status === 'success' ||
      row.translate_status === 'failed'
        ? row.translate_status
        : 'blocked',
  }), []);

  const loadSegments = useCallback(async (): Promise<boolean> => {
    try {
      const rows = await TauriAPI.listSegments(0, 200);
      const mapped = rows.map(mapDbSegment).filter((seg) => seg.text_raw.trim().length > 0);
      if (mapped.length > 0) {
        setSegments(mapped);
        setStatus('finished');
        return true;
      }
    } catch (err) {
      console.error('Load segments failed', err);
    }
    return false;
  }, [mapDbSegment]);

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
        const loaded = await loadSegments();
        if (!loaded) {
          setStatus('idle');
        }
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
  }, [loadSegments]);

  return {
    status, setStatus,
    segments, setSegments,
    devices, setDevices,
    selectedDevice, setSelectedDevice,
    elapsedTime, setElapsedTime,
    showEnglish, setShowEnglish,
    uiMode, setUiMode
  };
};
