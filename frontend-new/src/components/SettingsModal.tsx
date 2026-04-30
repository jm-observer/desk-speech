import React, { useEffect, useState } from 'react';
import { TauriAPI } from '../api/tauri-client';
import type { AppSettings } from '../api/tauri-client';
import { Button } from './ui/Button';

interface SettingsModalProps {
  open: boolean;
  onClose: () => void;
}

export const SettingsModal: React.FC<SettingsModalProps> = ({ open, onClose }) => {
  const [models, setModels] = useState<string[]>([]);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!open) return;
    const load = async () => {
      const [s, m] = await Promise.all([TauriAPI.getSettings(), TauriAPI.listLlmModels()]);
      setSettings(s);
      setModels(m.models);
    };
    load().catch((err) => console.error("Load settings failed", err));
  }, [open]);

  if (!open) return null;
  if (!settings) {
    return <div className="fixed inset-0 bg-black/30 backdrop-blur-[2px] z-40" />;
  }

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
      console.error("Apply settings failed", err);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="fixed inset-0 bg-black/30 backdrop-blur-[2px] z-40 flex items-center justify-center p-4" onClick={onClose}>
      <div className="w-full max-w-4xl bg-[var(--bg-card)] rounded-[20px] shadow-[var(--shadow-lg)] p-5" onClick={(e) => e.stopPropagation()}>
        <h3 className="text-[15px] font-semibold mb-4">识别参数设置</h3>
        <div className="grid grid-cols-2 gap-3">
          <input className="border rounded px-3 py-2" value={settings.threshold} onChange={(e) => patch('threshold', parseFloat(e.target.value) || 0)} placeholder="threshold" />
          <input className="border rounded px-3 py-2" value={settings.min_silence_duration} onChange={(e) => patch('min_silence_duration', parseFloat(e.target.value) || 0)} placeholder="min_silence_duration" />
          <input className="border rounded px-3 py-2" value={settings.min_speech_duration} onChange={(e) => patch('min_speech_duration', parseFloat(e.target.value) || 0)} placeholder="min_speech_duration" />
          <input className="border rounded px-3 py-2" value={settings.max_speech_duration} onChange={(e) => patch('max_speech_duration', parseFloat(e.target.value) || 0)} placeholder="max_speech_duration" />
          <input className="border rounded px-3 py-2" value={settings.num_threads} onChange={(e) => patch('num_threads', parseInt(e.target.value, 10) || 1)} placeholder="num_threads" />
          <input className="border rounded px-3 py-2" value={settings.provider_url} onChange={(e) => patch('provider_url', e.target.value)} placeholder="provider_url" />
          <input className="border rounded px-3 py-2 col-span-2" value={settings.api_key} onChange={(e) => patch('api_key', e.target.value)} placeholder="api_key" />
          <select className="border rounded px-3 py-2 col-span-2" value={settings.selected_model} onChange={(e) => patch('selected_model', e.target.value)}>
            {models.map((m) => <option key={m} value={m}>{m}</option>)}
          </select>
          <textarea className="border rounded px-3 py-2 col-span-2 min-h-[120px]" value={settings.prompt_template} onChange={(e) => patch('prompt_template', e.target.value)} />
        </div>
        <div className="mt-4 flex justify-end gap-2">
          <Button variant="outline" onClick={onClose}>取消</Button>
          <Button onClick={apply} disabled={saving}>{saving ? '应用中...' : '应用并重载模型'}</Button>
        </div>
      </div>
    </div>
  );
};
