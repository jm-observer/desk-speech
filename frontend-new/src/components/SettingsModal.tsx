import React, { useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { TauriAPI } from '../api/tauri-client';
import type { AppSettings } from '../api/tauri-client';
import { Button } from './ui/Button';

interface SettingsModalProps {
  open: boolean;
  onClose: () => void;
}

type Tab = 'vad' | 'asr' | 'llm';

export const SettingsModal: React.FC<SettingsModalProps> = ({ open, onClose }) => {
  const [models, setModels] = useState<string[]>([]);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [saving, setSaving] = useState(false);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>('vad');

  useEffect(() => {
    if (!open) return;
    const load = async () => {
      setLoading(true);
      setLoadError(null);
      try {
        const [s, m] = await Promise.allSettled([TauriAPI.getSettings(), TauriAPI.listLlmModels()]);
        if (s.status === 'fulfilled') {
          setSettings(s.value);
        } else {
          throw new Error('配置加载失败');
        }
        if (m.status === 'fulfilled') {
          setModels(m.value.models);
        } else {
          console.warn('Load llm models failed', m.reason);
          setModels([]);
        }
      } catch (err) {
        console.error('Load settings failed', err);
        setLoadError('配置加载失败，请重试');
      } finally {
        setLoading(false);
      }
    };
    load().catch((err) => console.error('Load settings failed unexpectedly', err));
  }, [open]);

  if (!open) return null;

  const patch = <K extends keyof AppSettings>(k: K, v: AppSettings[K]) => {
    setSettings((prev) => (prev ? { ...prev, [k]: v } : prev));
  };

  const apply = async () => {
    if (!settings) return;
    setSaving(true);
    try {
      await TauriAPI.applySettings(settings);
      onClose();
    } catch (err) {
      console.error('Apply settings failed', err);
    } finally {
      setSaving(false);
    }
  };

  const tabBtn = (value: Tab, label: string) => (
    <button
      className={`px-3 py-1.5 rounded-full text-[12px] ${tab === value ? 'bg-[var(--primary-soft)] text-[var(--primary-deep)]' : 'bg-[var(--bg-soft)] text-[var(--ink-3)]'}`}
      onClick={() => setTab(value)}
    >
      {label}
    </button>
  );

  return createPortal(
    <div className="fixed inset-0 bg-black/40 backdrop-blur-[2px] z-[9999] flex items-center justify-center p-4" onClick={onClose}>
      <div className="w-full max-w-4xl bg-[var(--bg-card)] rounded-[20px] shadow-[var(--shadow-lg)] p-5 relative" onClick={(e) => e.stopPropagation()}>
        <h3 className="text-[15px] font-semibold mb-4">识别参数设置</h3>

        {loading && (
          <div className="rounded-lg border border-[var(--line)] bg-[var(--bg-softer)] px-4 py-8 text-center text-[13px] text-[var(--ink-3)]">
            正在加载配置...
          </div>
        )}

        {!loading && loadError && (
          <div className="rounded-lg border border-[var(--danger)] bg-[var(--danger-soft)] px-4 py-4 text-[13px] text-[var(--danger)]">
            {loadError}
          </div>
        )}

        {!loading && !loadError && settings && (
          <>
        <div className="flex items-center gap-2 mb-4">
          {tabBtn('vad', 'VAD')}
          {tabBtn('asr', 'ASR')}
          {tabBtn('llm', 'LLM 润色')}
        </div>

        {tab === 'vad' && (
          <div className="grid grid-cols-2 gap-3">
            <input className="border rounded px-3 py-2" value={settings.threshold} onChange={(e) => patch('threshold', parseFloat(e.target.value) || 0)} placeholder="静音阈值 threshold (0~1)" />
            <input className="border rounded px-3 py-2" value={settings.min_silence_duration} onChange={(e) => patch('min_silence_duration', parseFloat(e.target.value) || 0)} placeholder="最小静音时长" />
            <input className="border rounded px-3 py-2" value={settings.min_speech_duration} onChange={(e) => patch('min_speech_duration', parseFloat(e.target.value) || 0)} placeholder="最小语音时长" />
            <input className="border rounded px-3 py-2" value={settings.max_speech_duration} onChange={(e) => patch('max_speech_duration', parseFloat(e.target.value) || 0)} placeholder="最大语音时长" />
          </div>
        )}

        {tab === 'asr' && (
          <div className="grid grid-cols-2 gap-3">
            <input className="border rounded px-3 py-2" value={settings.num_threads} onChange={(e) => patch('num_threads', parseInt(e.target.value, 10) || 1)} placeholder="线程数" />
            <input className="border rounded px-3 py-2" value={settings.provider_url} onChange={(e) => patch('provider_url', e.target.value)} placeholder="Provider URL" />
          </div>
        )}

        {tab === 'llm' && (
          <div className="grid grid-cols-2 gap-3">
            <input className="border rounded px-3 py-2 col-span-2" value={settings.api_key} onChange={(e) => patch('api_key', e.target.value)} placeholder="API Key" />
            <select className="border rounded px-3 py-2 col-span-2" value={settings.selected_model} onChange={(e) => patch('selected_model', e.target.value)}>
              {models.map((m) => <option key={m} value={m}>{m}</option>)}
            </select>
            <textarea className="border rounded px-3 py-2 col-span-2 min-h-[140px]" value={settings.prompt_template} onChange={(e) => patch('prompt_template', e.target.value)} placeholder="系统提示词" />
          </div>
        )}

        <div className="mt-4 flex justify-end gap-2">
          <Button variant="outline" onClick={onClose}>取消</Button>
          <Button onClick={apply} disabled={saving}>{saving ? '应用中...' : '应用并重载模型'}</Button>
        </div>
          </>
        )}

        {(loading || loadError) && (
          <div className="mt-4 flex justify-end gap-2">
            <Button variant="outline" onClick={onClose}>关闭</Button>
          </div>
        )}
      </div>
    </div>,
    document.body
  );
};
