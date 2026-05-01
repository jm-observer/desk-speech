import React, { useState } from 'react';
import { createPortal } from 'react-dom';
import { TauriAPI } from '../api/tauri-client';
import type { CorrectionRule } from '../api/tauri-client';
import { Button } from './ui/Button';

interface CorrectionModalProps {
  open: boolean;
  onClose: () => void;
}

export const CorrectionModal: React.FC<CorrectionModalProps> = ({ open, onClose }) => {
  const [rules, setRules] = useState<CorrectionRule[]>([]);
  const [dirtyRules, setDirtyRules] = useState<Record<number, CorrectionRule>>({});
  const [source, setSource] = useState('');
  const [target, setTarget] = useState('');
  const [priority, setPriority] = useState(100);

  const loadRules = async () => {
    const rows = await TauriAPI.listCorrectionRules();
    setRules(rows);
  };

  if (open && rules.length === 0) {
    loadRules().catch((err) => console.error('Load correction rules failed', err));
  }

  if (!open) return null;

  const addRule = async () => {
    if (!source.trim()) return;
    await TauriAPI.createCorrectionRule({ source: source.trim(), target: target.trim(), priority, enabled: true });
    await TauriAPI.reloadCorrectionRules();
    setSource('');
    setTarget('');
    setPriority(100);
    await loadRules();
  };

  const deleteRule = async (id: number) => {
    await TauriAPI.deleteCorrectionRule(id);
    await TauriAPI.reloadCorrectionRules();
    setDirtyRules((prev) => {
      const next = { ...prev };
      delete next[id];
      return next;
    });
    await loadRules();
  };

  const patchRule = (id: number, patch: Partial<CorrectionRule>) => {
    setDirtyRules((prev) => {
      const base = prev[id] ?? rules.find((r) => r.id === id);
      if (!base) return prev;
      return { ...prev, [id]: { ...base, ...patch } };
    });
  };

  const saveRule = async (id: number) => {
    const payload = dirtyRules[id];
    if (!payload) return;
    await TauriAPI.updateCorrectionRule(payload);
    await TauriAPI.reloadCorrectionRules();
    setDirtyRules((prev) => {
      const next = { ...prev };
      delete next[id];
      return next;
    });
    await loadRules();
  };

  const close = () => {
    setRules([]);
    setDirtyRules({});
    onClose();
  };

  return createPortal(
    <div className="fixed inset-0 bg-black/40 backdrop-blur-[2px] z-[9999] flex items-center justify-center p-4" onClick={close}>
      <div className="w-full max-w-3xl bg-[var(--bg-card)] rounded-[20px] shadow-[var(--shadow-lg)] p-5 relative" onClick={(e) => e.stopPropagation()}>
        <h3 className="text-[15px] font-semibold mb-4">词修正</h3>
        <div className="space-y-2 max-h-[320px] overflow-auto">
          {rules.map((rule) => {
            const draft = dirtyRules[rule.id] ?? rule;
            const hasChanges = Boolean(dirtyRules[rule.id]);
            return (
              <div key={rule.id} className="grid grid-cols-[1fr_24px_1fr_90px_80px_80px_auto] gap-2 items-center border rounded px-3 py-2">
                <input
                  className="border rounded px-2 py-1"
                  value={draft.source}
                  onChange={(e) => patchRule(rule.id, { source: e.target.value })}
                />
                <span className="text-center">→</span>
                <input
                  className="border rounded px-2 py-1"
                  value={draft.target}
                  onChange={(e) => patchRule(rule.id, { target: e.target.value })}
                />
                <input
                  className="border rounded px-2 py-1"
                  type="number"
                  value={draft.priority}
                  onChange={(e) => patchRule(rule.id, { priority: parseInt(e.target.value, 10) || 100 })}
                />
                <label className="text-[12px] flex items-center gap-1">
                  <input
                    type="checkbox"
                    checked={draft.enabled}
                    onChange={(e) => patchRule(rule.id, { enabled: e.target.checked })}
                  />
                  启用
                </label>
                <Button variant="outline" size="sm" disabled={!hasChanges} onClick={() => saveRule(rule.id)}>保存</Button>
                <Button variant="ghost" size="sm" onClick={() => deleteRule(rule.id)}>删除</Button>
              </div>
            );
          })}
        </div>
        <div className="grid grid-cols-[1fr_1fr_120px_auto] gap-2 mt-3">
          <input className="border rounded px-3 py-2" placeholder="原词" value={source} onChange={(e) => setSource(e.target.value)} />
          <input className="border rounded px-3 py-2" placeholder="替换为" value={target} onChange={(e) => setTarget(e.target.value)} />
          <input className="border rounded px-3 py-2" type="number" value={priority} onChange={(e) => setPriority(parseInt(e.target.value, 10) || 100)} />
          <Button variant="outline" onClick={addRule}>+ 添加规则</Button>
        </div>
        <div className="mt-4 flex justify-end">
          <Button onClick={close}>关闭</Button>
        </div>
      </div>
    </div>,
    document.body
  );
};
