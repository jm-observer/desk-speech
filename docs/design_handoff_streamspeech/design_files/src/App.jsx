// Main app — orchestrates state and the recording flow.
const { useState, useEffect, useRef, useCallback, useMemo } = React;

const TWEAK_DEFAULTS = /*EDITMODE-BEGIN*/{
  "primaryHue": 162,
  "density": "comfy",
  "viewMode": "card",
  "radius": "medium",
  "showEnglishDefault": true,
  "uiMode": "detailed"
}/*EDITMODE-END*/;

const DEFAULT_SETTINGS = {
  vadThreshold: 0.48,
  minSilence: 320,
  minSpeech: 240,
  maxSpeech: 30000,
  threads: 4,
  llmUrl: "https://api.anthropic.com/v1",
  apiKey: "sk-ant-•••••••••••••••",
  model: "claude-haiku-4-5",
  prompt: "请将下列识别结果润色为通顺、自然的中文，并去除口头语：\n{raw}",
};

function App() {
  const [tweaks, setTweak] = useTweaks(TWEAK_DEFAULTS);
  const toast = useToast();

  // Apply tweaks → CSS vars
  useEffect(() => {
    const root = document.documentElement;
    // Re-derive primary color from hue
    const h = tweaks.primaryHue;
    root.style.setProperty("--primary", `oklch(0.66 0.12 ${h})`);
    root.style.setProperty("--primary-deep", `oklch(0.56 0.13 ${h})`);
    root.style.setProperty("--primary-soft", `oklch(0.95 0.04 ${h})`);
    root.style.setProperty("--primary-softer", `oklch(0.975 0.02 ${h})`);
    // Density
    if (tweaks.density === "compact") {
      root.style.setProperty("--gap", "8px");
    } else if (tweaks.density === "comfy") {
      root.style.setProperty("--gap", "14px");
    } else {
      root.style.setProperty("--gap", "12px");
    }
    // Radius
    const radii = {
      sharp:  ["6px", "8px", "10px", "12px"],
      medium: ["8px", "12px", "16px", "20px"],
      soft:   ["12px", "16px", "20px", "26px"],
    };
    const [a, b, c, d] = radii[tweaks.radius] || radii.medium;
    root.style.setProperty("--radius-sm", a);
    root.style.setProperty("--radius", b);
    root.style.setProperty("--radius-lg", c);
    root.style.setProperty("--radius-xl", d);
  }, [tweaks]);

  // Core state
  const [modelReady, setModelReady] = useState(false);
  const [devices] = useState(MOCK_DEVICES);
  const [deviceId, setDeviceId] = useState("default");
  const [recording, setRecording] = useState(false);
  const [status, setStatus] = useState("initializing");
  const [elapsed, setElapsed] = useState(0);
  const [segments, setSegments] = useState(INITIAL_SEGMENTS);
  const [activeId, setActiveId] = useState(null);
  const [autoCopy, setAutoCopy] = useState(true);
  const [showEnglish, setShowEnglish] = useState(tweaks.showEnglishDefault);
  const simpleMode = tweaks.uiMode === "simple";

  // Audio playback (mock)
  const [playing, setPlaying] = useState(false);
  const [playPos, setPlayPos] = useState(0);
  const totalDuration = segments.length ? segments[segments.length - 1].end : 0;

  const [settingsOpen, setSettingsOpen] = useState(false);
  const [rulesOpen, setRulesOpen] = useState(false);
  const [settings, setSettings] = useState(DEFAULT_SETTINGS);
  const [rules, setRules] = useState(MOCK_RULES);

  // Boot — model init
  useEffect(() => {
    const t = setTimeout(() => {
      setModelReady(true);
      setStatus("idle");
    }, 1200);
    return () => clearTimeout(t);
  }, []);

  // Recording timer
  useEffect(() => {
    if (!recording) return;
    const t = setInterval(() => setElapsed((e) => e + 0.1), 100);
    return () => clearInterval(t);
  }, [recording]);

  // Playback ticker (mock)
  useEffect(() => {
    if (!playing) return;
    const t = setInterval(() => {
      setPlayPos((p) => {
        const next = p + 0.1;
        if (next >= totalDuration) { setPlaying(false); return totalDuration; }
        return next;
      });
    }, 100);
    return () => clearInterval(t);
  }, [playing, totalDuration]);

  // Auto-highlight active segment during playback
  useEffect(() => {
    if (!playing) return;
    const cur = segments.find((s) => playPos >= s.start && playPos < s.end);
    if (cur) setActiveId(cur.id);
  }, [playPos, playing, segments]);

  // Recording flow — drives the simulated segment stream
  const recordingTimers = useRef([]);
  const startRecording = useCallback(() => {
    if (recording || !modelReady) return;
    setRecording(true);
    setStatus("recording");
    setElapsed(0);
    // Don't clear past segments; append onto session, but mark fresh start
    let acc = segments.length ? segments[segments.length - 1].end + 0.5 : 0;
    let baseId = segments.length;

    MOCK_SEGMENTS.forEach((script, idx) => {
      const start = acc;
      const end = acc + script.duration;
      acc = end + (0.4 + Math.random() * 0.6);

      // Schedule appearance as "processing" after the segment "finishes"
      const arriveAt = (end - (segments.length ? segments[segments.length - 1].end : 0)) * 1000 + 600;
      const t1 = setTimeout(() => {
        setSegments((segs) => [...segs, {
          id: `live-${Date.now()}-${idx}`,
          start, end,
          raw: script.raw,
          polished: null,
          en: null,
          status: "processing",
          _script: script,
        }]);
      }, arriveAt);
      recordingTimers.current.push(t1);

      // Then resolve after LLM "polishing"
      const t2 = setTimeout(() => {
        setSegments((segs) => segs.map((s) => {
          if (s._script === script) {
            // auto-copy
            if (autoCopy) {
              try { navigator.clipboard?.writeText?.(script.polished); } catch(e) {}
            }
            return { ...s, polished: script.polished, en: script.en, status: "done" };
          }
          return s;
        }));
      }, arriveAt + 1100);
      recordingTimers.current.push(t2);
    });
  }, [recording, modelReady, segments, autoCopy]);

  const stopRecording = useCallback(() => {
    recordingTimers.current.forEach(clearTimeout);
    recordingTimers.current = [];
    setRecording(false);
    setStatus("processing");
    setTimeout(() => {
      setSegments((segs) => segs.map((s) => {
        if (s.status === "processing" && s._script) {
          return { ...s, polished: s._script.polished, en: s._script.en, status: "done" };
        }
        return s;
      }));
      setStatus("finished");
      toast("录音已停止，音频已就绪", "ok");
    }, 600);
  }, [toast]);

  const clearSession = useCallback(() => {
    if (recording) return;
    setSegments([]);
    setActiveId(null);
    setPlayPos(0);
    setPlaying(false);
    setElapsed(0);
    setStatus("idle");
    toast("结果已清空");
  }, [recording, toast]);

  // Result actions
  const copyChinese = useCallback(() => {
    const text = segments.filter((s) => s.status === "done").map((s) => s.polished || s.raw).join("\n");
    try { navigator.clipboard?.writeText?.(text); } catch(e) {}
    toast(`已复制中文（${segments.filter(s=>s.status==='done').length} 段）`);
  }, [segments, toast]);

  const copyEnglish = useCallback(() => {
    const text = segments.filter((s) => s.status === "done" && s.en).map((s) => s.en).join("\n");
    if (!text) { toast("暂无英文翻译可复制", "err"); return; }
    try { navigator.clipboard?.writeText?.(text); } catch(e) {}
    toast("已复制英文翻译");
  }, [segments, toast]);

  const copyWithTime = useCallback(() => {
    const text = segments.filter((s) => s.status === "done")
      .map((s) => `[${fmtTimeMs(s.start)} - ${fmtTimeMs(s.end)}] ${s.polished || s.raw}`).join("\n");
    try { navigator.clipboard?.writeText?.(text); } catch(e) {}
    toast("已复制带时间戳文本");
  }, [segments, toast]);

  const copySegZh = useCallback((seg) => {
    try { navigator.clipboard?.writeText?.(seg.polished || seg.raw); } catch(e) {}
  }, []);
  const copySegEn = useCallback((seg) => {
    try { navigator.clipboard?.writeText?.(seg.en || ""); } catch(e) {}
  }, []);
  const copySegBoth = useCallback((seg) => {
    const txt = [seg.polished || seg.raw, seg.en].filter(Boolean).join("\n");
    try { navigator.clipboard?.writeText?.(txt); } catch(e) {}
    toast("已复制中英文");
  }, [toast]);

  const exportSrt = useCallback(() => toast("已导出 SRT 字幕文件", "ok"), [toast]);
  const saveWav   = useCallback(() => toast("已保存录音 session.wav", "ok"), [toast]);
  const copySeg = useCallback((seg) => {
    try { navigator.clipboard?.writeText?.(seg.polished || seg.raw); } catch(e) {}
    toast("已复制此段");
  }, [toast]);
  const exportSegWav = useCallback((seg) => toast(`已导出 ${fmtTimeMs(seg.start)} 段音频`), [toast]);

  const seekTo = useCallback((t) => {
    setPlayPos(t);
    setPlaying(true);
  }, []);

  // Apply settings
  const applySettings = useCallback((s) => {
    setSettings(s);
    setSettingsOpen(false);
    setStatus("initializing");
    setModelReady(false);
    toast("参数已应用，模型重载中…", "info");
    setTimeout(() => { setModelReady(true); setStatus("idle"); toast("模型已就绪"); }, 1400);
  }, [toast]);

  const applyRules = useCallback((r) => {
    setRules(r);
    toast(`已保存 ${r.length} 条词修正规则`);
  }, [toast]);

  const doneSegments = segments.filter((s) => s.status === "done");

  return (
    <div style={{
      width: "100vw",
      height: "100vh",
      display: "flex",
      background: simpleMode ? "transparent" : "var(--bg-canvas)",
      overflow: "hidden",
      alignItems: simpleMode ? "flex-start" : "stretch",
      justifyContent: simpleMode ? "flex-start" : "stretch",
      padding: simpleMode ? 0 : 0,
    }}>
      {/* Left: control rail */}
      <aside style={{
        width: simpleMode ? 320 : 320,
        height: simpleMode ? "auto" : "100%",
        flexShrink: 0,
        background: simpleMode ? "var(--bg-card)" : "var(--bg-app)",
        borderRight: simpleMode ? "none" : "1px solid var(--line)",
        border: simpleMode ? "1px solid var(--line)" : "none",
        borderRadius: simpleMode ? 18 : 0,
        boxShadow: simpleMode ? "var(--shadow-lg)" : "none",
        display: "flex",
        flexDirection: "column",
        margin: simpleMode ? 16 : 0,
      }}>
        <ControlPanel
          status={status}
          recording={recording}
          deviceId={deviceId}
          devices={devices}
          onDeviceChange={setDeviceId}
          onStart={startRecording}
          onStop={stopRecording}
          onClear={clearSession}
          onOpenSettings={() => setSettingsOpen(true)}
          onOpenRules={() => setRulesOpen(true)}
          autoCopy={autoCopy}
          onAutoCopyChange={setAutoCopy}
          elapsed={elapsed}
          modelReady={modelReady}
          hasDevice={!!deviceId}
          showEnglish={showEnglish}
          onShowEnglishChange={setShowEnglish}
          simpleMode={simpleMode}
          onToggleMode={() => setTweak("uiMode", simpleMode ? "detailed" : "simple")}
        />
      </aside>

      {/* Right: results */}
      <main style={{
        flex: simpleMode ? "0 0 0" : 1,
        width: simpleMode ? 0 : "auto",
        minWidth: 0,
        display: simpleMode ? "none" : "flex",
        flexDirection: "column",
        background: "var(--bg-canvas)",
        overflow: "hidden",
      }}>
        {/* Top bar above results */}
        <header style={{
          display: "flex",
          alignItems: "center",
          gap: 10,
          padding: "20px 28px 14px",
          borderBottom: "1px solid var(--line)",
        }}>
          <div style={{ display: "flex", flexDirection: "column", gap: 2, flex: 1, minWidth: 0 }}>
            <div style={{ fontSize: 15, fontWeight: 600, color: "var(--ink)", letterSpacing: "-0.005em" }}>
              本次会话
            </div>
            <div style={{ fontSize: 12, color: "var(--ink-3)", display: "flex", gap: 12, alignItems: "center" }}>
              <span style={{ display: "inline-flex", alignItems: "center", gap: 4 }}>
                <Icon name="clock" size={11} />
                <span style={{ fontFamily: "var(--font-mono)" }}>{fmtTime(totalDuration)}</span>
              </span>
              <span style={{ width: 3, height: 3, borderRadius: "50%", background: "var(--ink-4)" }} />
              <span>{doneSegments.length} 段已识别</span>
              {recording && (
                <>
                  <span style={{ width: 3, height: 3, borderRadius: "50%", background: "var(--ink-4)" }} />
                  <span style={{ color: "var(--primary-deep)", display: "inline-flex", alignItems: "center", gap: 4 }}>
                    <span style={{ width: 6, height: 6, borderRadius: "50%", background: "var(--primary)", animation: "pulseDot 1.4s ease-in-out infinite" }} />
                    实时识别中
                  </span>
                </>
              )}
            </div>
          </div>

          {/* View mode segmented */}
          <div style={{ display: "flex", background: "var(--bg-soft)", borderRadius: 10, padding: 2 }}>
            {[
              { id: "bubble", label: "气泡" },
              { id: "card",   label: "卡片" },
              { id: "table",  label: "表格" },
            ].map((m) => {
              const active = tweaks.viewMode === m.id;
              return (
                <button
                  key={m.id}
                  onClick={() => setTweak("viewMode", m.id)}
                  style={{
                    height: 28,
                    padding: "0 12px",
                    fontSize: 12.5,
                    border: "none",
                    borderRadius: 8,
                    cursor: "pointer",
                    fontFamily: "inherit",
                    background: active ? "var(--bg-card)" : "transparent",
                    color: active ? "var(--ink)" : "var(--ink-3)",
                    fontWeight: active ? 500 : 400,
                    boxShadow: active ? "var(--shadow-sm)" : "none",
                    transition: "all 120ms ease",
                  }}
                >
                  {m.label}
                </button>
              );
            })}
          </div>
        </header>

        {/* Result actions */}
        <div style={{
          display: "flex",
          alignItems: "center",
          gap: 8,
          padding: "12px 28px",
          borderBottom: "1px solid var(--line)",
          background: "var(--bg-app)",
          flexWrap: "wrap",
        }}>
          <Button kind="outline" icon="copy" size="sm" onClick={copyChinese} disabled={!doneSegments.length}>
            复制中文
          </Button>
          <Button kind="outline" icon="languages" size="sm" onClick={copyEnglish} disabled={!doneSegments.some(s=>s.en)}>
            复制英文
          </Button>
          <Button kind="outline" icon="clock" size="sm" onClick={copyWithTime} disabled={!doneSegments.length}>
            含时间戳
          </Button>
          <Button kind="outline" icon="download" size="sm" onClick={exportSrt} disabled={!doneSegments.length}>
            导出 SRT
          </Button>
          <Button kind="outline" icon="save" size="sm" onClick={saveWav} disabled={!doneSegments.length}>
            保存音频
          </Button>
          <div style={{ flex: 1 }} />
          {!doneSegments.length && (
            <span style={{ fontSize: 11.5, color: "var(--ink-4)" }}>
              开始录音后将启用导出
            </span>
          )}
        </div>

        {/* Segment list */}
        <section style={{
          flex: 1,
          minHeight: 0,
          padding: "14px 24px",
          display: "flex",
          flexDirection: "column",
        }}>
          <SegmentList
            segments={segments}
            activeId={activeId}
            onSeek={seekTo}
            onCopyZh={copySegZh}
            onCopyEn={copySegEn}
            onCopyBoth={copySegBoth}
            onExportOne={exportSegWav}
            viewMode={tweaks.viewMode}
          />
        </section>

        {/* Audio player (sticky bottom) */}
        {totalDuration > 0 && (
          <div style={{ padding: "0 24px 22px" }}>
            <AudioPlayer
              duration={totalDuration}
              current={playPos}
              playing={playing}
              onSeek={(t) => setPlayPos(t)}
              onTogglePlay={() => setPlaying((p) => !p)}
              segments={segments}
              disabled={recording}
            />
          </div>
        )}
      </main>



      {/* Modals */}
      <SettingsModal
        open={settingsOpen}
        onClose={() => setSettingsOpen(false)}
        settings={settings}
        onApply={applySettings}
      />
      <RulesModal
        open={rulesOpen}
        onClose={() => setRulesOpen(false)}
        rules={rules}
        onChange={applyRules}
      />

      {/* Tweaks panel */}
      <TweaksPanel title="Tweaks">
        <TweakSection label="主题色调" />
        <TweakSlider label="主色相" value={tweaks.primaryHue} min={0} max={360} step={1}
                     onChange={(v) => setTweak("primaryHue", v)} />
        <TweakRadio  label="圆角" value={tweaks.radius}
                     options={["sharp", "medium", "soft"]}
                     onChange={(v) => setTweak("radius", v)} />
        <TweakSection label="排版" />
        <TweakRadio  label="信息密度" value={tweaks.density}
                     options={["compact", "regular", "comfy"]}
                     onChange={(v) => setTweak("density", v)} />
        <TweakSection label="界面模式" />
        <TweakRadio  label="模式" value={tweaks.uiMode}
                     options={["simple", "detailed"]}
                     onChange={(v) => setTweak("uiMode", v)} />
        <TweakSection label="结果展示" />
        <TweakRadio  label="分段视图" value={tweaks.viewMode}
                     options={["bubble", "card", "table"]}
                     onChange={(v) => setTweak("viewMode", v)} />
        <TweakToggle label="默认显示英文" value={tweaks.showEnglishDefault}
                     onChange={(v) => { setTweak("showEnglishDefault", v); setShowEnglish(v); }} />
      </TweaksPanel>
    </div>
  );
}

function Root() {
  return (
    <ToastProvider>
      <App />
    </ToastProvider>
  );
}

ReactDOM.createRoot(document.getElementById("root")).render(<Root />);
