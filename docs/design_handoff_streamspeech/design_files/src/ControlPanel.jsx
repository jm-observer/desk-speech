// Left control panel — supports simple + detailed modes.
const StatusChip = ({ status }) => {
  const map = {
    idle:        { label: "就绪", color: "neutral", dot: "#7a857f" },
    initializing:{ label: "模型加载中", color: "warn", dot: "#c98a2b" },
    recording:   { label: "正在录音", color: "mint", dot: "var(--primary)" },
    processing:  { label: "处理中",   color: "warn", dot: "#c98a2b" },
    error:       { label: "异常",     color: "danger", dot: "var(--danger)" },
    finished:    { label: "已完成",   color: "mint", dot: "var(--primary)" },
  };
  const m = map[status] || map.idle;
  return (
    <span style={{
      display: "inline-flex", alignItems: "center", gap: 6,
      padding: "5px 11px",
      borderRadius: 999,
      fontSize: 12,
      fontWeight: 500,
      background: m.color === "mint" ? "var(--primary-soft)" :
                  m.color === "warn" ? "#fef4e2" :
                  m.color === "danger" ? "var(--danger-soft)" : "var(--bg-soft)",
      color: m.color === "mint" ? "var(--primary-deep)" :
             m.color === "warn" ? "var(--warning)" :
             m.color === "danger" ? "var(--danger)" : "var(--ink-2)",
    }}>
      <span style={{
        width: 7, height: 7, borderRadius: "50%",
        background: m.dot,
        animation: status === "recording" ? "pulseDot 1.4s ease-in-out infinite" : undefined,
      }} />
      {m.label}
    </span>
  );
};

// Compact mode-switch — small pill at top right of the control panel
const ModeSwitch = ({ simpleMode, onToggle }) => (
  <button
    onClick={onToggle}
    title={simpleMode ? "切换到详细模式" : "切换到简洁模式"}
    style={{
      display: "inline-flex",
      alignItems: "center",
      gap: 6,
      padding: "5px 10px 5px 8px",
      borderRadius: 999,
      border: "1px solid var(--line)",
      background: "var(--bg-card)",
      color: "var(--ink-2)",
      fontSize: 11.5,
      cursor: "pointer",
      fontFamily: "inherit",
      transition: "all 140ms ease",
    }}
    onMouseEnter={(e) => { e.currentTarget.style.background = "var(--bg-soft)"; e.currentTarget.style.color = "var(--ink)"; }}
    onMouseLeave={(e) => { e.currentTarget.style.background = "var(--bg-card)"; e.currentTarget.style.color = "var(--ink-2)"; }}
  >
    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      {simpleMode
        ? <><path d="M3 12h18"/><path d="m13 5 7 7-7 7"/></>
        : <><path d="M21 12H3"/><path d="m11 5-7 7 7 7"/></>}
    </svg>
    {simpleMode ? "详细" : "简洁"}
  </button>
);

const RecordCard = ({ recording, modelReady, hasDevice, elapsed, onStart, onStop, large }) => (
  <div style={{
    display: "flex",
    flexDirection: "column",
    alignItems: "center",
    gap: large ? 22 : 16,
    padding: large ? "32px 28px 30px" : "20px 16px 22px",
    background: "linear-gradient(180deg, var(--primary-softer) 0%, transparent 100%)",
    borderRadius: large ? 24 : 18,
    border: "1px solid var(--primary-soft)",
    width: "100%",
  }}>
    <Waveform active={recording} intensity={recording ? 0.85 : 0.2} style={{ height: large ? 84 : 64 }} />

    <div style={{
      fontFamily: "var(--font-mono)",
      fontSize: large ? 56 : 30,
      fontWeight: 500,
      color: recording ? "var(--primary-deep)" : "var(--ink-3)",
      letterSpacing: "0.02em",
      fontVariantNumeric: "tabular-nums",
      display: "flex",
      alignItems: "center",
      gap: large ? 12 : 8,
      lineHeight: 1,
    }}>
      {recording && (
        <span style={{
          width: large ? 14 : 9, height: large ? 14 : 9, borderRadius: "50%",
          background: "var(--danger)",
          animation: "pulseDot 1s ease-in-out infinite",
        }} />
      )}
      {fmtTimeMs(elapsed)}
    </div>

    {!recording ? (
      <button
        onClick={onStart}
        disabled={!modelReady || !hasDevice}
        style={{
          width: "100%",
          height: large ? 60 : 46,
          borderRadius: large ? 16 : 12,
          border: "none",
          background: (!modelReady || !hasDevice) ? "#cdd5cf" : "linear-gradient(180deg, var(--primary) 0%, var(--primary-deep) 100%)",
          color: "#fff",
          fontSize: large ? 17 : 14.5,
          fontWeight: 600,
          cursor: (!modelReady || !hasDevice) ? "not-allowed" : "pointer",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          gap: 8,
          boxShadow: "0 4px 14px rgba(20,161,129,0.22)",
          transition: "transform 120ms ease, box-shadow 120ms ease",
          fontFamily: "inherit",
          letterSpacing: "0.04em",
        }}
        onMouseEnter={(e) => { if (modelReady && hasDevice) { e.currentTarget.style.transform = "translateY(-1px)"; e.currentTarget.style.boxShadow = "0 6px 18px rgba(20,161,129,0.32)"; }}}
        onMouseLeave={(e) => { e.currentTarget.style.transform = "translateY(0)"; e.currentTarget.style.boxShadow = "0 4px 14px rgba(20,161,129,0.22)"; }}
      >
        <Icon name="mic" size={large ? 20 : 17} />
        开始录音
      </button>
    ) : (
      <button
        onClick={onStop}
        style={{
          width: "100%",
          height: large ? 60 : 46,
          borderRadius: large ? 16 : 12,
          border: "1px solid var(--danger)",
          background: "var(--bg-card)",
          color: "var(--danger)",
          fontSize: large ? 17 : 14.5,
          fontWeight: 600,
          cursor: "pointer",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          gap: 8,
          fontFamily: "inherit",
          transition: "background 120ms ease",
          letterSpacing: "0.04em",
        }}
        onMouseEnter={(e) => e.currentTarget.style.background = "var(--danger-soft)"}
        onMouseLeave={(e) => e.currentTarget.style.background = "var(--bg-card)"}
      >
        <Icon name="stop" size={large ? 18 : 15} />
        停止录音
      </button>
    )}

    <div style={{ fontSize: 11, color: "var(--ink-4)", display: "flex", alignItems: "center", gap: 4 }}>
      快捷键
      <kbd style={kbdStyle}>⌘</kbd>
      <kbd style={kbdStyle}>R</kbd>
    </div>
  </div>
);

const ControlPanel = ({
  status, recording, deviceId, devices, onDeviceChange,
  onStart, onStop, onClear, onOpenSettings, onOpenRules,
  autoCopy, onAutoCopyChange, elapsed, modelReady, hasDevice,
  showEnglish, onShowEnglishChange,
  simpleMode, onToggleMode,
}) => {

  // SIMPLE MODE — small compact widget
  if (simpleMode) {
    return (
      <div style={{
        display: "flex",
        flexDirection: "column",
        gap: 12,
        padding: 14,
      }}>
        {/* Top bar: logo + status + mode switch */}
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <div style={{
            width: 22, height: 22, borderRadius: 6,
            background: "linear-gradient(135deg, var(--primary) 0%, var(--primary-deep) 100%)",
            display: "flex", alignItems: "center", justifyContent: "center", color: "#fff",
            flexShrink: 0,
          }}>
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.6" strokeLinecap="round">
              <path d="M4 12c1.5 0 1.5-5 3-5s1.5 10 3 10 1.5-13 3-13 1.5 8 3 8 1.5-3 3-3" />
            </svg>
          </div>
          <div style={{ fontSize: 12.5, fontWeight: 600, color: "var(--ink)", letterSpacing: "-0.005em" }}>
            StreamSpeech
          </div>
          <div style={{ flex: 1 }} />
          <StatusChip status={status} />
          <ModeSwitch simpleMode={true} onToggle={onToggleMode} />
        </div>

        {/* Compact record card */}
        <div style={{
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          gap: 10,
          padding: "14px 14px 12px",
          background: "linear-gradient(180deg, var(--primary-softer) 0%, transparent 100%)",
          borderRadius: 14,
          border: "1px solid var(--primary-soft)",
        }}>
          <Waveform active={recording} intensity={recording ? 0.85 : 0.2} style={{ height: 44 }} />
          <div style={{
            fontFamily: "var(--font-mono)",
            fontSize: 24,
            fontWeight: 500,
            color: recording ? "var(--primary-deep)" : "var(--ink-3)",
            letterSpacing: "0.02em",
            fontVariantNumeric: "tabular-nums",
            display: "flex",
            alignItems: "center",
            gap: 6,
            lineHeight: 1,
          }}>
            {recording && (
              <span style={{
                width: 8, height: 8, borderRadius: "50%",
                background: "var(--danger)",
                animation: "pulseDot 1s ease-in-out infinite",
              }} />
            )}
            {fmtTimeMs(elapsed)}
          </div>
          {!recording ? (
            <button
              onClick={onStart}
              disabled={!modelReady || !hasDevice}
              style={{
                width: "100%",
                height: 38,
                borderRadius: 10,
                border: "none",
                background: (!modelReady || !hasDevice) ? "#cdd5cf" : "linear-gradient(180deg, var(--primary) 0%, var(--primary-deep) 100%)",
                color: "#fff",
                fontSize: 13.5,
                fontWeight: 600,
                cursor: (!modelReady || !hasDevice) ? "not-allowed" : "pointer",
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
                gap: 6,
                boxShadow: "0 2px 8px rgba(20,161,129,0.20)",
                fontFamily: "inherit",
              }}
            >
              <Icon name="mic" size={14} />
              开始录音
            </button>
          ) : (
            <button
              onClick={onStop}
              style={{
                width: "100%",
                height: 38,
                borderRadius: 10,
                border: "1px solid var(--danger)",
                background: "var(--bg-card)",
                color: "var(--danger)",
                fontSize: 13.5,
                fontWeight: 600,
                cursor: "pointer",
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
                gap: 6,
                fontFamily: "inherit",
              }}
            >
              <Icon name="stop" size={13} />
              停止录音
            </button>
          )}
        </div>
      </div>
    );
  }

  // DETAILED MODE — full panel
  return (
    <div style={{
      display: "flex",
      flexDirection: "column",
      gap: 22,
      padding: "26px 24px",
      height: "100%",
      overflowY: "auto",
    }}>
      {/* Logo + title row + mode switch */}
      <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
        <div style={{
          width: 32, height: 32,
          borderRadius: 9,
          background: "linear-gradient(135deg, var(--primary) 0%, var(--primary-deep) 100%)",
          display: "flex", alignItems: "center", justifyContent: "center",
          color: "#fff",
          boxShadow: "0 2px 8px rgba(20,161,129,0.25)",
        }}>
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round">
            <path d="M4 12c1.5 0 1.5-5 3-5s1.5 10 3 10 1.5-13 3-13 1.5 8 3 8 1.5-3 3-3" />
          </svg>
        </div>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: 14.5, fontWeight: 600, color: "var(--ink)", letterSpacing: "-0.005em" }}>
            StreamSpeech
          </div>
          <div style={{ fontSize: 11, color: "var(--ink-4)", letterSpacing: "0.02em" }}>
            离线语音识别 · v1.4.0
          </div>
        </div>
        <ModeSwitch simpleMode={false} onToggle={onToggleMode} />
      </div>

      <div style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
        <StatusChip status={status} />
        {!modelReady && <span style={{ fontSize: 11.5, color: "var(--ink-4)" }}>正在加载离线模型…</span>}
      </div>

      <div style={{ display: "flex", flexDirection: "column", gap: 7 }}>
        <label style={{ fontSize: 11.5, color: "var(--ink-3)", fontWeight: 500, letterSpacing: "0.04em", textTransform: "uppercase" }}>
          输入设备
        </label>
        <Dropdown
          value={deviceId}
          options={devices}
          onChange={onDeviceChange}
          disabled={recording}
          icon="device"
          placeholder="未检测到麦克风"
        />
      </div>

      <RecordCard
        recording={recording}
        modelReady={modelReady}
        hasDevice={hasDevice}
        elapsed={elapsed}
        onStart={onStart}
        onStop={onStop}
      />

      <div style={{ display: "flex", flexDirection: "column", gap: 14, padding: "0 2px" }}>
        <Switch
          checked={autoCopy}
          onChange={onAutoCopyChange}
          label="自动复制到剪贴板"
          sublabel="新分段识别完成后自动写入"
        />
        <Switch
          checked={showEnglish}
          onChange={onShowEnglishChange}
          label="显示英文翻译"
          sublabel="LLM 同步生成对照翻译"
        />
      </div>

      <div style={{ flex: 1 }} />

      <div style={{
        display: "grid",
        gridTemplateColumns: "1fr 1fr",
        gap: 8,
        paddingTop: 16,
        borderTop: "1px solid var(--line)",
      }}>
        <Button kind="soft" icon="clear" onClick={onClear} disabled={recording}>清空结果</Button>
        <Button kind="soft" icon="wand" onClick={onOpenRules}>词修正</Button>
        <Button kind="soft" icon="settings" onClick={onOpenSettings} disabled={recording} style={{ gridColumn: "span 2" }}>识别参数设置</Button>
      </div>
    </div>
  );
};

const kbdStyle = {
  fontFamily: "var(--font-mono)",
  fontSize: 10,
  padding: "1px 6px",
  borderRadius: 4,
  background: "var(--bg-card)",
  border: "1px solid var(--line)",
  color: "var(--ink-3)",
  marginLeft: 2,
};

window.ControlPanel = ControlPanel;
