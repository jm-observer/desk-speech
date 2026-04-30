// Settings modal — VAD / Recognizer / LLM
const SettingsModal = ({ open, onClose, settings, onApply }) => {
  const [draft, setDraft] = React.useState(settings);
  const [refreshing, setRefreshing] = React.useState(false);
  React.useEffect(() => { if (open) setDraft(settings); }, [open, settings]);

  const update = (k, v) => setDraft((d) => ({ ...d, [k]: v }));

  const refresh = () => {
    setRefreshing(true);
    setTimeout(() => setRefreshing(false), 900);
  };

  const Section = ({ title, icon, desc, children }) => (
    <div style={{ display: "flex", flexDirection: "column", gap: 12, padding: "16px 0", borderBottom: "1px solid var(--line)" }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <Icon name={icon} size={14} style={{ color: "var(--primary-deep)" }} />
        <div style={{ fontSize: 13, fontWeight: 600, color: "var(--ink)" }}>{title}</div>
        {desc && <span style={{ fontSize: 11.5, color: "var(--ink-4)" }}>· {desc}</span>}
      </div>
      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "14px 16px" }}>
        {children}
      </div>
    </div>
  );

  return (
    <Modal
      open={open}
      onClose={onClose}
      title="识别参数设置"
      subtitle="参数应用后会触发模型重载"
      icon="settings"
      width={620}
      footer={
        <>
          <Button kind="ghost" onClick={onClose}>取消</Button>
          <Button kind="primary" icon="check" onClick={() => onApply(draft)}>应用并重载</Button>
        </>
      }
    >
      <Section title="VAD 语音活动检测" icon="sparkles">
        <Field label="检测阈值" hint="越低越灵敏，过低会误触发" span={2}>
          <Slider value={draft.vadThreshold} min={0} max={1} step={0.01} onChange={(v) => update("vadThreshold", v)} />
        </Field>
        <Field label="最小静音时长">
          <TextInput type="number" value={draft.minSilence} onChange={(v) => update("minSilence", parseInt(v) || 0)} suffix="ms" monospace />
        </Field>
        <Field label="最小语音时长">
          <TextInput type="number" value={draft.minSpeech} onChange={(v) => update("minSpeech", parseInt(v) || 0)} suffix="ms" monospace />
        </Field>
        <Field label="最大语音时长" span={2}>
          <TextInput type="number" value={draft.maxSpeech} onChange={(v) => update("maxSpeech", parseInt(v) || 0)} suffix="ms" monospace />
        </Field>
      </Section>

      <Section title="识别器" icon="mic">
        <Field label="工作线程数" hint="建议不超过 CPU 物理核心数" span={2}>
          <Slider value={draft.threads} min={1} max={16} step={1} onChange={(v) => update("threads", v)} suffix=" 线程" />
        </Field>
      </Section>

      <Section title="LLM 润色与翻译" icon="wand">
        <Field label="Provider 接口" span={2}>
          <TextInput value={draft.llmUrl} onChange={(v) => update("llmUrl", v)} placeholder="https://api.example.com/v1" monospace />
        </Field>
        <Field label="API Key" span={2}>
          <TextInput type="password" value={draft.apiKey} onChange={(v) => update("apiKey", v)} placeholder="sk-..." monospace />
        </Field>
        <Field label="模型" span={2}>
          <div style={{ display: "flex", gap: 8 }}>
            <div style={{ flex: 1 }}>
              <Dropdown value={draft.model} options={MOCK_MODELS.map((m) => ({ id: m, name: m }))} onChange={(v) => update("model", v)} />
            </div>
            <Button kind="outline" icon="refresh" onClick={refresh} disabled={refreshing}>
              {refreshing ? "刷新中…" : "刷新模型"}
            </Button>
          </div>
        </Field>
        <Field label="Prompt 模板" hint="支持 {raw} 占位，输出会替换识别原文" span={2}>
          <textarea
            value={draft.prompt}
            onChange={(e) => update("prompt", e.target.value)}
            rows={4}
            style={{
              width: "100%",
              padding: "10px 12px",
              borderRadius: 9,
              border: "1px solid var(--line)",
              background: "var(--bg-card)",
              fontSize: 12.5,
              fontFamily: "var(--font-mono)",
              outline: "none",
              resize: "vertical",
              lineHeight: 1.55,
            }}
            onFocus={(e) => { e.target.style.borderColor = "var(--primary)"; e.target.style.boxShadow = "0 0 0 3px var(--primary-softer)"; }}
            onBlur={(e) => { e.target.style.borderColor = "var(--line)"; e.target.style.boxShadow = "none"; }}
          />
        </Field>
      </Section>
    </Modal>
  );
};

window.SettingsModal = SettingsModal;
