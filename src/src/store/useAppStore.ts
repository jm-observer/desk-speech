import { useState, useEffect, useCallback } from 'react';
import { TauriAPI, type SegmentDiscardedEvent, type SegmentUpdatedEvent, type QualityFilterConfig, type ValidationErrorsResponse } from '../api/tauri-client';
import type { Segment } from '../api/tauri-client';
import { listen } from '@tauri-apps/api/event';

export type AppStatus = 'idle' | 'initializing' | 'recording' | 'processing' | 'error' | 'finished';

export const useAppStore = () => {
  const [status, setStatus] = useState<AppStatus>('initializing');
  const [segments, setSegments] = useState<Segment[]>([]);
  const [devices, setDevices] = useState<{ label: string; value: string }[]>([]);
  const [selectedDevice, setSelectedDevice] = useState<string>('');
  const [showEnglish, setShowEnglish] = useState(true);
  const [uiMode, setUiMode] = useState<'detailed' | 'simple'>('detailed');
  const [isInitialized, setIsInitialized] = useState(false);
  const [qualityFilterConfig, setQualityFilterConfig] = useState<QualityFilterConfig | null>(null);
  const [configValidationError, setConfigValidationError] = useState<string | null>(null);

  const deleteSegment = useCallback(async (segment: Segment) => {
    const segmentId = segment.segment_id ?? segment.id;
    if (segmentId === null || segmentId === undefined) {
      console.error('Cannot delete segment: missing segment_id');
      return;
    }
    try {
      await TauriAPI.deleteSegment(segmentId);
      setSegments((prev) => {
        const key = segment.segment_id != null ? `seg-${segment.segment_id}` : `db-${segment.id}`;
        return prev.filter((s) => {
          if (s.segment_id != null && `seg-${s.segment_id}` === key) return false;
          if (s.id != null && `db-${s.id}` === key) return false;
          return true;
        }).sort((a, b) => {
          if (a.wall_start !== b.wall_start) {
            return a.wall_start.localeCompare(b.wall_start);
          }
          return a.start - b.start;
        });
      });
    } catch (err) {
      console.error('Delete segment failed', err);
      throw err;
    }
  }, []);

  const mapDbSegment = useCallback((row: Record<string, unknown>): Segment => ({
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
      console.debug('[segments][db-load]', {
        rowCount: rows.length,
        mappedCount: mapped.length,
        firstSegmentId: mapped[0]?.segment_id ?? null,
        lastSegmentId: mapped[mapped.length - 1]?.segment_id ?? null,
        firstRevision: mapped[0]?.revision ?? null,
        lastRevision: mapped[mapped.length - 1]?.revision ?? null,
      });
      if (mapped.length > 0) {
        setSegments((prev) => {
          const merged = new Map<string, Segment>();
          // Helper to get key consistent with App.tsx
          const getInternalKey = (s: Segment) => {
            if (s.segment_id !== null && s.segment_id !== undefined) {
              return `seg-${s.segment_id}`;
            }
            if (s.id !== null && s.id !== undefined) {
              return `db-${s.id}`;
            }
            return `ts-${s.start.toFixed(3)}`;
          };

          prev.forEach(s => merged.set(getInternalKey(s), s));
          mapped.forEach(s => {
            const key = getInternalKey(s);
            if (!merged.has(key)) {
              merged.set(key, s);
            }
          });

          return Array.from(merged.values()).sort((a, b) => {
            if (a.wall_start !== b.wall_start) {
              return a.wall_start.localeCompare(b.wall_start);
            }
            return a.start - b.start;
          });
        });
        setStatus((prev) => (prev === 'initializing' || prev === 'idle' ? 'finished' : prev));
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
    let unsubscribeSegmentDiscarded: (() => void) | null = null;
    let unsubscribeSegmentUpdated: (() => void) | null = null;
    let unsubscribeConfigUpdated: (() => void) | null = null;
    let unsubscribeConfigValidationError: (() => void) | null = null;

    const init = async () => {
      try {
        const devices = await TauriAPI.listDevices();
        if (canceled) return;
        setDevices(devices.map(d => ({ label: d.is_default ? `${d.name} (Default)` : d.name, value: d.name })));
        
        const selected = await TauriAPI.getSelectedDevice();
        if (canceled) return;
        if (selected) setSelectedDevice(selected);

        // Load quality filter config
        try {
          const config = await TauriAPI.getQualityFilterConfig();
          if (canceled) return;
          setQualityFilterConfig(config);
        } catch (err) {
          console.error('Failed to load quality filter config', err);
        }

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
          setStatus((prev) => (prev === 'initializing' ? 'idle' : prev));
        }
      } else if (res.status === 2) {
        setStatus('error');
      } else {
        initTimer = window.setTimeout(pollInit, 500);
      }
    };

    const runInit = async () => {
      await init();
      setIsInitialized(true);
    };
    runInit();

    // Subscribe to segment_discarded events (Plan 3)
    void listen<SegmentDiscardedEvent>('segment_discarded', (event) => {
      if (canceled) return;
      const { revision, segment_id } = event.payload;
      console.debug('[segment_discarded]', { revision, segment_id, reason: event.payload.reason });

      // Remove segment from list by revision
      setSegments((prev) => {
        const filtered = prev.filter(s => {
          // Remove if segment_id matches
          if (segment_id !== null && s.segment_id === segment_id) {
            return false;
          }
          // Also remove if revision matches (for segments without segment_id)
          if (s.revision !== undefined && revision !== undefined && s.revision === revision) {
            return false;
          }
          return true;
        });

        return filtered.sort((a, b) => {
          if (a.wall_start !== b.wall_start) {
            return a.wall_start.localeCompare(b.wall_start);
          }
          return a.start - b.start;
        });
      });
    })
      .then((unlisten) => {
        if (canceled) {
          unlisten();
          return;
        }
        unsubscribeSegmentDiscarded = unlisten;
      })
      .catch((err) => {
        console.error('Subscribe segment_discarded failed', err);
      });

    void listen<SegmentUpdatedEvent>('segment_updated', (event) => {
      if (canceled) return;
      const row = event.payload;
      const next = mapDbSegment(row as unknown as Record<string, unknown>);
      if (next.revision === undefined) {
        return;
      }
      console.debug('[segment_updated]', { revision: next.revision, segmentId: next.segment_id });
      setSegments((prev) => {
        const exists = prev.some((segment) => segment.revision === next.revision);
        const updated = exists
          ? prev.map((segment) =>
              segment.revision === next.revision ? { ...segment, ...next } : segment,
            )
          : [...prev, next];

        return updated.sort((a, b) => {
          if (a.wall_start !== b.wall_start) {
            return a.wall_start.localeCompare(b.wall_start);
          }
          return a.start - b.start;
        });
      });
    })
      .then((unlisten) => {
        if (canceled) {
          unlisten();
          return;
        }
        unsubscribeSegmentUpdated = unlisten;
      })
      .catch((err) => {
        console.error('Subscribe segment_updated failed', err);
      });

    // Subscribe to quality filter config updated events
    void listen<QualityFilterConfig>('quality_filter_config_updated', (event) => {
      if (canceled) return;
      console.debug('[quality_filter_config_updated]', event.payload);
      setQualityFilterConfig(event.payload);
    })
      .then((unlisten) => {
        if (canceled) {
          unlisten();
          return;
        }
        unsubscribeConfigUpdated = unlisten;
      })
      .catch((err) => {
        console.error('Subscribe quality_filter_config_updated failed', err);
      });

    // Subscribe to validation errors
    void listen<ValidationErrorsResponse>('quality_filter_config_validation_error', (event) => {
      if (canceled) return;
      const errors = event.payload.errors;
      const messages = errors.map(e => `${e.field}: ${e.message}`).join('; ');
      setConfigValidationError(messages);
      console.warn('[quality_filter_config_validation_error]', messages);
      // Clear after 5 seconds
      setTimeout(() => setConfigValidationError(null), 5000);
    })
      .then((unlisten) => {
        if (canceled) {
          unlisten();
          return;
        }
        unsubscribeConfigValidationError = unlisten;
      })
      .catch((err) => {
        console.error('Subscribe quality_filter_config_validation_error failed', err);
      });

    return () => {
      canceled = true;
      if (initTimer !== null) {
        window.clearTimeout(initTimer);
      }
      if (typeof unsubscribeSegmentDiscarded === 'function') {
        unsubscribeSegmentDiscarded();
      }
      if (typeof unsubscribeSegmentUpdated === 'function') {
        unsubscribeSegmentUpdated();
      }
      if (typeof unsubscribeConfigUpdated === 'function') {
        unsubscribeConfigUpdated();
      }
      if (typeof unsubscribeConfigValidationError === 'function') {
        unsubscribeConfigValidationError();
      }
    };
  }, [loadSegments]);

  return {
    status, setStatus,
    segments, setSegments,
    loadSegments,
    deleteSegment,
    devices, setDevices,
    selectedDevice, setSelectedDevice,
    showEnglish, setShowEnglish,
    uiMode, setUiMode,
    isInitialized,
    qualityFilterConfig, setQualityFilterConfig,
    configValidationError, setConfigValidationError,
  };
};
