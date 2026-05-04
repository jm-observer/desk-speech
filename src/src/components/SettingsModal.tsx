import React, { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { TauriAPI } from '../api/tauri-client';
import type { AppSettings } from '../api/tauri-client';
import { Button } from './ui/Button';

interface SettingsModalProps {
  open: boolean;
  onClose: () => void;
}

type Tab = 'vad' | 'asr' | 'llm';
type FieldKey = keyof AppSettings;
type FieldErrorMap = Partial<Record<FieldKey, string>>;
type InputKind = 'number' | 'text' | 'password' | 'select' | 'textarea';

interface FieldDefinition {
  key: FieldKey;
  tab: Tab;
  label: string;
  description: string;
  helpText?: string;
  placeholder?: string;
  kind: InputKind;
  fullWidth?: boolean;
  min?: number;
  max?: number;
  step?: number;
  options?: Array<{ value: AppSettings['auto_copy_mode']; label: string }>;
}

const TAB_LABELS: Record<Tab, string> = {
  vad: 'VAD 分段',
  asr: 'ASR 识别',
  llm: 'LLM 后处理',
};

const TAB_DESCRIPTIONS: Record<Tab, string> = {
  vad: '控制系统如何判断“开始说话”和“停止说话”，会直接影响分段频率、漏切和误切。',
  asr: '控制本地识别模型的并行度，主要影响 CPU 占用与识别速度。',
  llm: '配置远程模型接入与文本后处理策略，影响润色、翻译和自动复制结果。',
};

const FIELD_DEFINITIONS: FieldDefinition[] = [
  {
    key: 'threshold',
    tab: 'vad',
    label: '静音阈值',
    description: '控制语音与静音的分界敏感度。',
    helpText: '取值范围 0~1，推荐 0.2。值越小越容易判定为语音，值越大越容易截断轻声。',
    placeholder: '例如 0.2',
    kind: 'number',
    min: 0,
    max: 1,
    step: 0.01,
  },
  {
    key: 'min_silence_duration',
    tab: 'vad',
    label: '最小静音时长',
    description: '连续静音达到该时长后，才会切分一段语音。',
    helpText: '单位：秒，推荐 0.2。',
    placeholder: '例如 0.2',
    kind: 'number',
    min: 0,
    step: 0.1,
  },
  {
    key: 'min_speech_duration',
    tab: 'vad',
    label: '最小语音时长',
    description: '短于该时长的语音片段会被忽略或合并。',
    helpText: '单位：秒，推荐 0.2。',
    placeholder: '例如 0.2',
    kind: 'number',
    min: 0,
    step: 0.1,
  },
  {
    key: 'max_speech_duration',
    tab: 'vad',
    label: '最大语音时长',
    description: '单段语音的最长持续时间，超出后强制切段。',
    helpText: '单位：秒，必须大于 0，推荐 10。',
    placeholder: '例如 10',
    kind: 'number',
    min: 0,
    step: 0.5,
  },
  {
    key: 'num_threads',
    tab: 'asr',
    label: '识别线程数',
    description: '用于加载和运行识别模型的线程数量。',
    helpText: '取值范围 1~16，推荐按 CPU 核心数调整。线程过高可能导致系统卡顿。',
    placeholder: '例如 4',
    kind: 'number',
    min: 1,
    max: 16,
    step: 1,
  },
  {
    key: 'auto_copy_mode',
    tab: 'llm',
    label: '自动复制策略',
    description: '决定识别完成后自动写入剪贴板的文本版本。',
    kind: 'select',
    fullWidth: true,
    options: [
      { value: 'off', label: '关闭' },
      { value: 'english', label: '自动复制英文' },
      { value: 'optimized_zh', label: '自动复制中文' },
    ],
  },
  {
    key: 'provider_url',
    tab: 'llm',
    label: '模型服务地址',
    description: '兼容 OpenAI 接口的服务根地址。',
    helpText: '示例：https://api.openai.com/v1',
    placeholder: 'https://api.openai.com/v1',
    kind: 'text',
  },
  {
    key: 'api_key',
    tab: 'llm',
    label: 'API Key',
    description: '调用模型服务所需的鉴权令牌。',
    helpText: '默认遮罩显示，点击右侧按钮可临时查看。',
    placeholder: 'sk-...',
    kind: 'password',
  },
  {
    key: 'selected_model',
    tab: 'llm',
    label: '默认模型',
    description: '用于润色与翻译的模型名称。',
    kind: 'select',
  },
  {
    key: 'optimize_prompt_template',
    tab: 'llm',
    label: '润色提示词',
    description: '定义纠错、去噪、补标点的系统提示词。',
    helpText: '协议要求：需返回 JSON 且包含 `text_optimized` 字段。',
    placeholder: '请输入润色用的 Prompt...',
    kind: 'textarea',
    fullWidth: true,
  },
  {
    key: 'translate_prompt_template',
    tab: 'llm',
    label: '翻译提示词',
    description: '定义中文转英文的系统提示词。',
    helpText: '协议要求：需返回 JSON 且包含 `text_english` 字段。',
    placeholder: '请输入翻译用的 Prompt...',
    kind: 'textarea',
    fullWidth: true,
  },
];

const FIELD_TAB_MAP: Record<FieldKey, Tab> = FIELD_DEFINITIONS.reduce(
  (acc, field) => ({ ...acc, [field.key]: field.tab }),
  {} as Record<FieldKey, Tab>,
);

const FIRST_ERROR_FIELD_ORDER: FieldKey[] = FIELD_DEFINITIONS.map((field) => field.key);

const validateField = (key: FieldKey, value: AppSettings[FieldKey]): string | null => {
  switch (key) {
    case 'threshold':
      return typeof value !== 'number' || Number.isNaN(value) || value <= 0 || value >= 1
        ? '静音阈值必须大于 0 且小于 1'
        : null;
    case 'min_silence_duration':
      return typeof value !== 'number' || Number.isNaN(value) || value < 0 ? '最小静音时长不能小于 0' : null;
    case 'min_speech_duration':
      return typeof value !== 'number' || Number.isNaN(value) || value < 0 ? '最小语音时长不能小于 0' : null;
    case 'max_speech_duration':
      return typeof value !== 'number' || Number.isNaN(value) || value <= 0 ? '最大语音时长必须大于 0' : null;
    case 'num_threads':
      return typeof value !== 'number' || Number.isNaN(value) || value < 1 || value > 16
        ? '线程数必须在 1 到 16 之间'
        : null;
    case 'provider_url':
      return typeof value !== 'string' || value.trim() === '' ? '模型服务地址不能为空' : null;
    case 'optimize_prompt_template':
      return typeof value !== 'string' || value.trim() === '' ? '润色提示词不能为空' : null;
    case 'translate_prompt_template':
      return typeof value !== 'string' || value.trim() === '' ? '翻译提示词不能为空' : null;
    default:
      return null;
  }
};

const validateSettings = (settings: AppSettings): FieldErrorMap => {
  const nextErrors: FieldErrorMap = {};
  for (const field of FIELD_DEFINITIONS) {
    const error = validateField(field.key, settings[field.key]);
    if (error) {
      nextErrors[field.key] = error;
    }
  }
  return nextErrors;
};

const getFirstErrorField = (errors: FieldErrorMap): FieldKey | null => {
  for (const key of FIRST_ERROR_FIELD_ORDER) {
    if (errors[key]) {
      return key;
    }
  }
  return null;
};

const getErrorMessage = (err: unknown): string => {
  if (typeof err === 'string') {
    return err;
  }
  if (err && typeof err === 'object' && 'message' in err && typeof err.message === 'string') {
    return err.message;
  }
  return '保存失败，请重试';
};

const mapBackendError = (message: string): { field?: FieldKey; form?: string } => {
  if (message.includes('threshold')) {
    return { field: 'threshold' };
  }
  if (message.includes('min_silence_duration')) {
    return { field: 'min_silence_duration' };
  }
  if (message.includes('min_speech_duration')) {
    return { field: 'min_speech_duration' };
  }
  if (message.includes('max_speech_duration')) {
    return { field: 'max_speech_duration' };
  }
  if (message.includes('num_threads')) {
    return { field: 'num_threads' };
  }
  if (message.includes('provider_url')) {
    return { field: 'provider_url' };
  }
  if (message.includes('optimize_prompt_template')) {
    return { field: 'optimize_prompt_template' };
  }
  if (message.includes('translate_prompt_template')) {
    return { field: 'translate_prompt_template' };
  }
  if (message.includes('Cannot change settings while recording')) {
    return { form: '录音进行中不可修改设置，请停止录音后再保存。' };
  }
  if (message.includes('Models are still loading')) {
    return { form: '识别模型仍在加载中，请稍后再保存。' };
  }
  return { form: `保存失败：${message}` };
};

const SettingsField = ({
  field,
  error,
  helpText,
  children,
}: {
  field: FieldDefinition;
  error?: string;
  helpText?: string;
  children: React.ReactNode;
}) => (
  <div className={`flex flex-col gap-2 ${field.fullWidth ? 'md:col-span-2' : ''}`}>
    <div className="flex flex-col gap-1">
      <label
        htmlFor={`settings-field-${field.key}`}
        className={`text-[13px] font-semibold ${error ? 'text-[var(--danger)]' : 'text-[var(--ink-1)]'}`}
      >
        {field.label}
      </label>
      <p className="text-[11px] leading-tight text-[var(--ink-4)]">{field.description}</p>
    </div>
    {children}
    {helpText && <p className="text-[11px] text-[var(--ink-3)]">{helpText}</p>}
    {error && <p className="text-[11px] font-medium text-[var(--danger)]">{error}</p>}
  </div>
);

export const SettingsModal: React.FC<SettingsModalProps> = ({ open, onClose }) => {
  const [models, setModels] = useState<string[]>([]);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [saving, setSaving] = useState(false);
  const [loading, setLoading] = useState(false);
  const [loadingModels, setLoadingModels] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [modelError, setModelError] = useState<string | null>(null);
  const [formError, setFormError] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>('vad');
  const [showApiKey, setShowApiKey] = useState(false);
  const [errors, setErrors] = useState<FieldErrorMap>({});
  const [touched, setTouched] = useState<Partial<Record<FieldKey, boolean>>>({});
  const scrollRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) {
      return;
    }

    const loadSettings = async () => {
      setErrors({});
      setTouched({});
      setFormError(null);
      setShowApiKey(false);
      setTab('vad');
      setLoading(true);
      setLoadError(null);
      try {
        const nextSettings = await TauriAPI.getSettings();
        setSettings(nextSettings);
      } catch (err) {
        console.error('Load settings failed', err);
        setLoadError('配置加载失败，请重试。');
      } finally {
        setLoading(false);
      }
    };

    const loadModels = async () => {
      setLoadingModels(true);
      setModelError(null);
      try {
        const response = await TauriAPI.listLlmModels();
        setModels(response.models);
      } catch (err) {
        console.warn('Load llm models failed', err);
        setModelError('模型列表加载失败，可保留当前已保存值继续保存其他配置。');
      } finally {
        setLoadingModels(false);
      }
    };

    void loadSettings();
    void loadModels();
  }, [open]);

  const focusField = (key: FieldKey) => {
    setTab(FIELD_TAB_MAP[key]);
    requestAnimationFrame(() => {
      const element = document.getElementById(`settings-field-${key}`);
      element?.scrollIntoView({ behavior: 'smooth', block: 'center' });
      element?.focus();
    });
  };

  const updateFieldError = <K extends FieldKey>(key: K, value: AppSettings[K]) => {
    setErrors((prev) => {
      const next = { ...prev };
      const error = validateField(key, value);
      if (error) {
        next[key] = error;
      } else {
        delete next[key];
      }
      return next;
    });
  };

  const patch = <K extends FieldKey>(key: K, value: AppSettings[K]) => {
    setSettings((prev) => (prev ? { ...prev, [key]: value } : prev));
    setFormError(null);
    if (touched[key]) {
      updateFieldError(key, value);
    }
  };

  const markTouched = (key: FieldKey) => {
    setTouched((prev) => ({ ...prev, [key]: true }));
    if (settings) {
      updateFieldError(key, settings[key]);
    }
  };

  const apply = async () => {
    if (!settings) {
      return;
    }

    const nextErrors = validateSettings(settings);
    setErrors(nextErrors);
    setTouched(
      FIELD_DEFINITIONS.reduce(
        (acc, field) => ({ ...acc, [field.key]: true }),
        {} as Partial<Record<FieldKey, boolean>>,
      ),
    );

    const firstErrorField = getFirstErrorField(nextErrors);
    if (firstErrorField) {
      setFormError('请先修正标红字段后再保存。');
      focusField(firstErrorField);
      return;
    }

    setSaving(true);
    setFormError(null);

    try {
      await TauriAPI.applySettings(settings);
      onClose();
    } catch (err) {
      console.error('Apply settings failed', err);
      const message = getErrorMessage(err);
      const mapped = mapBackendError(message);
      if (mapped.field) {
        const fieldKey = mapped.field;
        const fieldMessage = validateField(fieldKey, settings[fieldKey]) ?? message;
        setErrors((prev) => ({ ...prev, [fieldKey]: fieldMessage }));
        setFormError('保存失败，请检查对应字段。');
        focusField(fieldKey);
      } else {
        setFormError(mapped.form ?? '保存失败，请重试。');
        scrollRef.current?.scrollTo({ top: 0, behavior: 'smooth' });
      }
    } finally {
      setSaving(false);
    }
  };

  const renderField = (field: FieldDefinition) => {
    if (!settings) {
      return null;
    }

    const error = errors[field.key];
    const commonClassName = `w-full rounded-lg border px-3 py-2 text-[14px] outline-none transition-colors ${
      error
        ? 'border-[var(--danger)] bg-[var(--danger-soft)]/40 text-[var(--ink-1)]'
        : 'border-[var(--line)] bg-[var(--bg-card)] text-[var(--ink-1)] focus:border-[var(--primary)]'
    }`;

    let helpText = field.helpText;
    if (field.key === 'selected_model') {
      if (loadingModels) {
        helpText = '正在加载模型列表，当前值会保留。';
      } else if (modelError) {
        helpText = modelError;
      } else if (settings.selected_model.trim()) {
        helpText = `当前已保存值：${settings.selected_model}`;
      } else {
        helpText = '可从远端模型列表中选择默认模型。';
      }
    }

    if (field.key === 'api_key') {
      return (
        <SettingsField key={field.key} field={field} error={error} helpText={helpText}>
          <div className="relative">
            <input
              id={`settings-field-${field.key}`}
              type={showApiKey ? 'text' : 'password'}
              className={`${commonClassName} pr-20`}
              value={settings.api_key}
              onBlur={() => markTouched(field.key)}
              onChange={(event) => patch('api_key', event.target.value)}
              placeholder={field.placeholder}
            />
            <button
              type="button"
              className="absolute right-2 top-1/2 -translate-y-1/2 rounded-md px-2 py-1 text-[11px] text-[var(--ink-3)] hover:bg-[var(--bg-soft)] hover:text-[var(--ink-1)]"
              onClick={() => setShowApiKey((prev) => !prev)}
            >
              {showApiKey ? '隐藏' : '显示'}
            </button>
          </div>
        </SettingsField>
      );
    }

    if (field.key === 'selected_model') {
      const modelOptions = models.includes(settings.selected_model)
        ? models
        : settings.selected_model.trim()
          ? [settings.selected_model, ...models]
          : models;

      return (
        <SettingsField key={field.key} field={field} error={error} helpText={helpText}>
          <select
            id={`settings-field-${field.key}`}
            className={commonClassName}
            value={settings.selected_model}
            onBlur={() => markTouched(field.key)}
            onChange={(event) => patch('selected_model', event.target.value)}
            disabled={loadingModels && modelOptions.length === 0}
          >
            {!settings.selected_model.trim() && <option value="">请选择模型</option>}
            {modelOptions.map((model) => (
              <option key={model} value={model}>
                {model}
              </option>
            ))}
          </select>
        </SettingsField>
      );
    }

    if (field.kind === 'select' && field.options) {
      return (
        <SettingsField key={field.key} field={field} error={error} helpText={helpText}>
          <select
            id={`settings-field-${field.key}`}
            className={commonClassName}
            value={settings.auto_copy_mode}
            onBlur={() => markTouched(field.key)}
            onChange={(event) => patch('auto_copy_mode', event.target.value as AppSettings['auto_copy_mode'])}
          >
            {field.options.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        </SettingsField>
      );
    }

    if (field.kind === 'textarea') {
      return (
        <SettingsField key={field.key} field={field} error={error} helpText={helpText}>
          <textarea
            id={`settings-field-${field.key}`}
            className={`${commonClassName} min-h-[120px]`}
            value={settings[field.key] as string}
            onBlur={() => markTouched(field.key)}
            onChange={(event) => patch(field.key, event.target.value as AppSettings[typeof field.key])}
            placeholder={field.placeholder}
          />
        </SettingsField>
      );
    }

    if (field.kind === 'number') {
      return (
        <SettingsField key={field.key} field={field} error={error} helpText={helpText}>
          <input
            id={`settings-field-${field.key}`}
            type="number"
            className={commonClassName}
            value={settings[field.key] as number}
            min={field.min}
            max={field.max}
            step={field.step}
            onBlur={() => markTouched(field.key)}
            onChange={(event) => patch(field.key, Number(event.target.value) as AppSettings[typeof field.key])}
            placeholder={field.placeholder}
          />
        </SettingsField>
      );
    }

    return (
      <SettingsField key={field.key} field={field} error={error} helpText={helpText}>
        <input
          id={`settings-field-${field.key}`}
          type="text"
          className={commonClassName}
          value={settings[field.key] as string}
          onBlur={() => markTouched(field.key)}
          onChange={(event) => patch(field.key, event.target.value as AppSettings[typeof field.key])}
          placeholder={field.placeholder}
        />
      </SettingsField>
    );
  };

  const currentFields = FIELD_DEFINITIONS.filter((field) => field.tab === tab);

  const renderTabButton = (value: Tab) => (
    <button
      key={value}
      type="button"
      className={`rounded-full px-4 py-1.5 text-[13px] font-medium transition-colors ${
        tab === value
          ? 'bg-[var(--primary-soft)] text-[var(--primary-deep)]'
          : 'bg-[var(--bg-soft)] text-[var(--ink-3)] hover:bg-[var(--bg-line)]'
      }`}
      onClick={() => setTab(value)}
    >
      {TAB_LABELS[value]}
    </button>
  );

  if (!open) {
    return null;
  }

  return createPortal(
    <div className="fixed inset-0 z-[9999] flex items-center justify-center bg-black/40 p-4 backdrop-blur-[2px]" onClick={onClose}>
      <div
        className="flex max-h-[90vh] w-full max-w-4xl flex-col rounded-[24px] bg-[var(--bg-card)] shadow-[var(--shadow-lg)]"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="flex flex-col gap-1 p-6 pb-0">
          <h3 className="text-[18px] font-bold text-[var(--ink-1)]">识别参数设置</h3>
          <p className="text-[13px] text-[var(--ink-3)]">参数影响分段、识别性能和 LLM 后处理效果，修改后会重新加载模型。</p>
        </div>

        <div ref={scrollRef} className="flex-1 overflow-y-auto p-6">
          {loading && (
            <div className="rounded-xl border border-[var(--line)] bg-[var(--bg-softer)] px-4 py-12 text-center text-[14px] text-[var(--ink-3)]">
              正在加载配置...
            </div>
          )}

          {!loading && loadError && (
            <div className="rounded-xl border border-[var(--danger)] bg-[var(--danger-soft)] px-4 py-6 text-center text-[14px] text-[var(--danger)]">
              {loadError}
            </div>
          )}

          {!loading && !loadError && settings && (
            <>
              {formError && (
                <div className="mb-4 rounded-xl border border-[var(--danger)] bg-[var(--danger-soft)] px-4 py-3 text-[13px] text-[var(--danger)]">
                  {formError}
                </div>
              )}
              <div className="mb-6 flex flex-wrap items-center gap-3">{(['vad', 'asr', 'llm'] as Tab[]).map(renderTabButton)}</div>

              <div className="mb-8 rounded-xl border border-[var(--line)] bg-[var(--bg-softer)] p-4 text-[13px] leading-relaxed text-[var(--ink-3)]">
                {TAB_DESCRIPTIONS[tab]}
              </div>

              <div className="grid grid-cols-1 gap-x-8 gap-y-8 md:grid-cols-2">{currentFields.map(renderField)}</div>
            </>
          )}
        </div>

        <div className="rounded-b-[24px] border-t border-[var(--line)] bg-[var(--bg-card)] p-6 pt-2">
          <div className="flex flex-col gap-4 md:flex-row md:items-center md:justify-between">
            <p className="text-[11px] font-medium italic text-[var(--danger)]">保存后会重新加载识别模型，录音中不可修改。</p>
            <div className="flex gap-3">
              <Button variant="outline" onClick={onClose} className="px-6">
                取消
              </Button>
              <Button onClick={apply} disabled={saving} className="bg-[var(--primary)] px-6 text-white">
                {saving ? '应用中...' : '应用并保存'}
              </Button>
            </div>
          </div>
        </div>
      </div>
    </div>,
    document.body,
  );
};
