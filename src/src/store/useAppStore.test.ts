import { describe, it, expect, vi, beforeEach } from 'vitest';
import { listen } from '@tauri-apps/api/event';
import { type SegmentDiscardedEvent, type Segment } from '../api/tauri-client';
import { useAppStore } from './useAppStore';

// Mock Tauri API
vi.mock('../api/tauri-client', () => ({
  TauriAPI: {
    listDevices: vi.fn(),
    getSelectedDevice: vi.fn(),
    getInitStatus: vi.fn(),
    getQualityFilterConfig: vi.fn(),
  },
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(),
}));

// Mock window.setTimeout/clearTimeout
const mockSetTimeout = vi.fn((_fn: () => void, ms: number) => ms);
const mockClearTimeout = vi.fn();
vi.stubGlobal('setTimeout', mockSetTimeout);
vi.stubGlobal('clearTimeout', mockClearTimeout);

describe('useAppStore - segment_discarded event handling', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('should subscribe to segment_discarded events on initialization', () => {
    const unsubscribeMock = vi.fn();
    vi.mocked(listen).mockReturnValue(unsubscribeMock as any);

    useAppStore();

    expect(listen).toHaveBeenCalledWith('segment_discarded', expect.any(Function));
  });

  it('should remove segment when segment_id matches', () => {
    const segments: Segment[] = [
      { id: 1, segment_id: 100, revision: 1, start: 0, end: 1, wall_start: '2026-01-01 00:00:00', wall_end: '2026-01-01 00:00:01', text_raw: 'hello', optimize_status: 'success', translate_status: 'success' },
      { id: 2, segment_id: 200, revision: 2, start: 1, end: 2, wall_start: '2026-01-01 00:00:01', wall_end: '2026-01-01 00:00:02', text_raw: 'world', optimize_status: 'success', translate_status: 'success' },
    ];

    const store = useAppStore();
    store.setSegments(segments);

    // Simulate segment_discarded event
    const listener = vi.mocked(listen).mock.calls[0][1] as (event: { payload: SegmentDiscardedEvent }) => void;
    listener({
      payload: {
        revision: 1,
        segment_id: 100,
        decision: 'DISCARD',
        reason: 'Rule: filler word',
        source: 'rule',
        confidence: null,
        occurred_at_ms: Date.now(),
      },
    } as any);

    expect(store.segments.length).toBe(1);
    expect(store.segments[0].segment_id).toBe(200);
  });

  it('should remove segment when revision matches (fallback)', () => {
    const segments: Segment[] = [
      { id: 1, segment_id: null, revision: 1, start: 0, end: 1, wall_start: '2026-01-01 00:00:00', wall_end: '2026-01-01 00:00:01', text_raw: 'hello', optimize_status: 'success', translate_status: 'success' },
      { id: 2, segment_id: 200, revision: 2, start: 1, end: 2, wall_start: '2026-01-01 00:00:01', wall_end: '2026-01-01 00:00:02', text_raw: 'world', optimize_status: 'success', translate_status: 'success' },
    ];

    const store = useAppStore();
    store.setSegments(segments);

    const listener = vi.mocked(listen).mock.calls[0][1] as (event: { payload: SegmentDiscardedEvent }) => void;
    listener({
      payload: {
        revision: 1,
        segment_id: 0,
        decision: 'DISCARD',
        reason: 'LLM: low info',
        source: 'llm',
        confidence: 0.85,
        occurred_at_ms: Date.now(),
      },
    } as any);

    expect(store.segments.length).toBe(1);
    expect(store.segments[0].revision).toBe(2);
  });

  it('should handle unknown revision gracefully', () => {
    const segments: Segment[] = [
      { id: 1, segment_id: 100, revision: 1, start: 0, end: 1, wall_start: '2026-01-01 00:00:00', wall_end: '2026-01-01 00:00:01', text_raw: 'hello', optimize_status: 'success', translate_status: 'success' },
    ];

    const store = useAppStore();
    store.setSegments(segments);

    const listener = vi.mocked(listen).mock.calls[0][1] as (event: { payload: SegmentDiscardedEvent }) => void;
    // segment_id 999 does not exist
    listener({
      payload: {
        revision: 999,
        segment_id: 999,
        decision: 'DISCARD',
        reason: 'test',
        source: 'rule',
        confidence: null,
        occurred_at_ms: Date.now(),
      },
    } as any);

    // Segment should remain unchanged
    expect(store.segments.length).toBe(1);
  });

  it('should sort segments after removal', () => {
    const segments: Segment[] = [
      { id: 1, segment_id: 100, revision: 1, start: 0, end: 1, wall_start: '2026-01-01 00:00:00', wall_end: '2026-01-01 00:00:01', text_raw: 'a', optimize_status: 'success', translate_status: 'success' },
      { id: 2, segment_id: 200, revision: 2, start: 1, end: 2, wall_start: '2026-01-01 00:00:01', wall_end: '2026-01-01 00:00:02', text_raw: 'b', optimize_status: 'success', translate_status: 'success' },
      { id: 3, segment_id: 300, revision: 3, start: 2, end: 3, wall_start: '2026-01-01 00:00:02', wall_end: '2026-01-01 00:00:03', text_raw: 'c', optimize_status: 'success', translate_status: 'success' },
    ];

    const store = useAppStore();
    store.setSegments(segments);

    const listener = vi.mocked(listen).mock.calls[0][1] as (event: { payload: SegmentDiscardedEvent }) => void;
    listener({
      payload: {
        revision: 2,
        segment_id: 200,
        decision: 'DISCARD',
        reason: 'test',
        source: 'llm',
        confidence: 0.7,
        occurred_at_ms: Date.now(),
      },
    } as any);

    expect(store.segments.length).toBe(2);
    expect(store.segments[0].segment_id).toBe(100);
    expect(store.segments[1].segment_id).toBe(300);
  });
});
