// Card-style segment list — each card shows Chinese + English with per-language copy.
const SegmentCard = ({ seg, active, onSeek, onCopyZh, onCopyEn, onCopyBoth, onExportOne, viewMode }) => {
  const [hover, setHover] = React.useState(false);
  const [showOriginal, setShowOriginal] = React.useState(false);
  const [zhCopied, setZhCopied] = React.useState(false);
  const [enCopied, setEnCopied] = React.useState(false);
  const isProcessing = seg.status === "processing";

  const flash = (setter) => { setter(true); setTimeout(() => setter(false), 1400); };

  // table view kept minimal
  if (viewMode === "table") {
    return (
      <tr style={{
        background: active ? "var(--primary-softer)" : "transparent",
        borderBottom: "1px solid var(--line)",
      }}>
        <td style={{ padding: "10px 12px", fontFamily: "var(--font-mono)", fontSize: 12, color: "var(--ink-3)", whiteSpace: "nowrap" }}>
          {fmtTimeMs(seg.start)}
        </td>
        <td style={{ padding: "10px 12px", fontSize: 13.5, color: "var(--ink)", lineHeight: 1.55 }}>
          {isProcessing ? <SkeletonLine /> : (seg.polished || seg.raw)}
        </td>
        <td style={{ padding: "10px 12px", fontSize: 12.5, color: "var(--ink-3)", lineHeight: 1.55 }}>
          {seg.en || <span style={{ color: "var(--ink-4)" }}>—</span>}
        </td>
        <td style={{ padding: "10px 12px", textAlign: "right", whiteSpace: "nowrap" }}>
          <IconButton icon="play" size={26} onClick={() => onSeek(seg.start)} title="跳转播放" />
          <IconButton icon="copy" size={26} onClick={() => onCopyBoth(seg)} title="复制中英文" />
        </td>
      </tr>
    );
  }

  // bubble view (compact, single-language)
  if (viewMode === "bubble") {
    return (
      <div
        onMouseEnter={() => setHover(true)}
        onMouseLeave={() => setHover(false)}
        style={{ display: "flex", flexDirection: "column", gap: 6, animation: "fadeUp 280ms ease" }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <button onClick={() => onSeek(seg.start)} style={tsBtn(active)}>
            {active && <span style={{ width: 6, height: 6, borderRadius: "50%", background: "var(--primary)", animation: "pulseDot 1.6s ease-in-out infinite" }} />}
            {fmtTimeMs(seg.start)}
          </button>
          <span style={{ fontSize: 11, color: "var(--ink-4)", fontFamily: "var(--font-mono)" }}>
            {(seg.end - seg.start).toFixed(1)}s
          </span>
          {isProcessing && <Tag color="warn">润色中</Tag>}
          <div style={{ flex: 1 }} />
          <div style={{ display: "flex", gap: 2, opacity: hover ? 1 : 0, transition: "opacity 140ms ease" }}>
            <IconButton icon="copy" size={26} onClick={() => onCopyBoth(seg)} title="复制" />
          </div>
        </div>
        <div style={{
          background: active ? "linear-gradient(180deg, var(--primary-softer) 0%, var(--primary-soft) 100%)" : "var(--bg-card)",
          border: `1px solid ${active ? "var(--primary-soft)" : "var(--line)"}`,
          borderRadius: 14,
          padding: "12px 14px",
          boxShadow: active ? "none" : "var(--shadow-sm)",
        }}>
          {isProcessing ? <SkeletonLine /> : (
            <div style={{ fontSize: 14.5, lineHeight: 1.7, color: "var(--ink)", wordBreak: "break-word" }}>
              {seg.polished || seg.raw}
            </div>
          )}
        </div>
      </div>
    );
  }

  // CARD view (default)
  return (
    <div
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        background: "var(--bg-card)",
        border: `1px solid ${active ? "var(--primary)" : "var(--line)"}`,
        borderRadius: 16,
        padding: 0,
        boxShadow: active ? "0 0 0 3px var(--primary-softer), var(--shadow-md)" : "var(--shadow-sm)",
        overflow: "hidden",
        transition: "all 200ms ease",
        animation: "fadeUp 280ms ease",
      }}
    >
      {/* card header */}
      <div style={{
        display: "flex",
        alignItems: "center",
        gap: 8,
        padding: "10px 14px",
        background: active ? "var(--primary-softer)" : "var(--bg-softer)",
        borderBottom: "1px solid var(--line)",
      }}>
        <button onClick={() => onSeek(seg.start)} style={tsBtn(active)}>
          {active ? (
            <span style={{ width: 6, height: 6, borderRadius: "50%", background: "var(--primary)", animation: "pulseDot 1.6s ease-in-out infinite" }} />
          ) : (
            <Icon name="play" size={10} style={{ marginRight: -1 }} />
          )}
          {fmtTimeMs(seg.start)}
          <span style={{ opacity: 0.5 }}>—</span>
          {fmtTimeMs(seg.end)}
        </button>
        <span style={{
          fontSize: 11,
          color: "var(--ink-4)",
          fontFamily: "var(--font-mono)",
          padding: "2px 6px",
          background: "var(--bg-card)",
          borderRadius: 4,
        }}>
          {(seg.end - seg.start).toFixed(1)}s
        </span>
        {isProcessing && (
          <Tag color="warn">
            <span style={{
              width: 8, height: 8, border: "1.5px solid currentColor",
              borderTopColor: "transparent", borderRadius: "50%",
              animation: "spin 0.8s linear infinite", display: "inline-block",
            }} />
            润色中
          </Tag>
        )}
        <div style={{ flex: 1 }} />
        <IconButton icon="scissors" size={26} onClick={() => onExportOne(seg)} title="导出此段音频" />
      </div>

      {/* Chinese block */}
      <div style={{
        display: "grid",
        gridTemplateColumns: "32px 1fr auto",
        gap: 10,
        alignItems: "flex-start",
        padding: "14px 14px 12px",
      }}>
        <span style={langBadge("zh")}>中</span>
        <div style={{
          fontSize: 14.5,
          lineHeight: 1.75,
          color: "var(--ink)",
          letterSpacing: "0.005em",
          wordBreak: "break-word",
          paddingTop: 4,
        }}>
          {isProcessing ? <SkeletonLine /> : (seg.polished || seg.raw)}
        </div>
        <button
          onClick={() => { if (isProcessing) return; onCopyZh(seg); flash(setZhCopied); }}
          disabled={isProcessing}
          style={copyBtnStyle(zhCopied, isProcessing)}
          title="复制中文"
        >
          <Icon name={zhCopied ? "check" : "copy"} size={12} />
          {zhCopied ? "已复制" : "复制"}
        </button>
      </div>

      {/* English block */}
      {(!isProcessing || true) && (
        <div style={{
          display: "grid",
          gridTemplateColumns: "32px 1fr auto",
          gap: 10,
          alignItems: "flex-start",
          padding: "12px 14px 14px",
          borderTop: "1px dashed var(--line)",
          background: "linear-gradient(180deg, transparent, rgba(0,0,0,0.012))",
        }}>
          <span style={langBadge("en")}>EN</span>
          <div style={{
            fontSize: 13,
            lineHeight: 1.65,
            color: seg.en ? "var(--ink-2)" : "var(--ink-4)",
            fontFamily: "var(--font-display)",
            fontStyle: seg.en ? "normal" : "italic",
            wordBreak: "break-word",
            paddingTop: 3,
          }}>
            {isProcessing ? <SkeletonLine width="60%" /> : (seg.en || "等待翻译…")}
          </div>
          <button
            onClick={() => { if (!seg.en) return; onCopyEn(seg); flash(setEnCopied); }}
            disabled={!seg.en}
            style={copyBtnStyle(enCopied, !seg.en)}
            title="复制英文"
          >
            <Icon name={enCopied ? "check" : "copy"} size={12} />
            {enCopied ? "已复制" : "复制"}
          </button>
        </div>
      )}

      {/* original (raw) toggle */}
      {!isProcessing && seg.polished && seg.raw && seg.polished !== seg.raw && (
        <div style={{
          padding: "0 14px 10px",
          borderTop: showOriginal ? "1px dashed var(--line)" : "none",
        }}>
          {!showOriginal ? (
            <button onClick={() => setShowOriginal(true)} style={subtleLinkBtn}>
              <Icon name="chevron-down" size={11} /> 查看原文
            </button>
          ) : (
            <div style={{ paddingTop: 10 }}>
              <div style={{ fontSize: 10.5, color: "var(--ink-4)", letterSpacing: "0.06em", textTransform: "uppercase", marginBottom: 4 }}>
                原始识别
              </div>
              <div style={{ fontSize: 12.5, color: "var(--ink-3)", lineHeight: 1.65 }}>{seg.raw}</div>
              <button onClick={() => setShowOriginal(false)} style={{ ...subtleLinkBtn, marginTop: 6 }}>
                <Icon name="chevron-up" size={11} /> 收起
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
};

const tsBtn = (active) => ({
  display: "inline-flex",
  alignItems: "center",
  gap: 5,
  padding: "3px 9px",
  background: active ? "var(--primary-soft)" : "var(--bg-card)",
  color: active ? "var(--primary-deep)" : "var(--ink-2)",
  fontFamily: "var(--font-mono)",
  fontSize: 11.5,
  border: active ? "1px solid var(--primary-soft)" : "1px solid var(--line)",
  borderRadius: 999,
  cursor: "pointer",
  fontWeight: 500,
});

const langBadge = (lang) => ({
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  width: 26,
  height: 22,
  borderRadius: 6,
  fontSize: lang === "zh" ? 12 : 10.5,
  fontWeight: 600,
  fontFamily: lang === "en" ? "var(--font-display)" : "var(--font-sans)",
  letterSpacing: "0.02em",
  background: lang === "zh" ? "var(--primary-soft)" : "var(--bg-soft)",
  color: lang === "zh" ? "var(--primary-deep)" : "var(--ink-2)",
  flexShrink: 0,
  marginTop: 2,
});

const copyBtnStyle = (copied, disabled) => ({
  display: "inline-flex",
  alignItems: "center",
  gap: 4,
  padding: "5px 9px",
  borderRadius: 7,
  border: `1px solid ${copied ? "var(--primary)" : "var(--line)"}`,
  background: copied ? "var(--primary-soft)" : "var(--bg-card)",
  color: copied ? "var(--primary-deep)" : "var(--ink-2)",
  fontSize: 11.5,
  fontFamily: "inherit",
  cursor: disabled ? "not-allowed" : "pointer",
  opacity: disabled ? 0.4 : 1,
  flexShrink: 0,
  transition: "all 140ms ease",
});

const subtleLinkBtn = {
  padding: 0,
  background: "transparent",
  border: "none",
  fontSize: 11.5,
  color: "var(--ink-3)",
  cursor: "pointer",
  display: "inline-flex",
  alignItems: "center",
  gap: 3,
  fontFamily: "inherit",
};

const SkeletonLine = ({ width = "75%" }) => (
  <div style={{
    height: 13,
    borderRadius: 4,
    background: "linear-gradient(90deg, var(--bg-soft) 0%, #e8eee9 50%, var(--bg-soft) 100%)",
    backgroundSize: "200% 100%",
    animation: "shimmer 1.4s linear infinite",
    width,
  }} />
);

const SegmentList = ({ segments, activeId, onSeek, onCopyZh, onCopyEn, onCopyBoth, onExportOne, viewMode = "card" }) => {
  const ref = React.useRef(null);
  React.useEffect(() => {
    if (ref.current) ref.current.scrollTop = ref.current.scrollHeight;
  }, [segments.length]);

  if (segments.length === 0) {
    return (
      <div style={{
        flex: 1,
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        gap: 12,
        color: "var(--ink-4)",
      }}>
        <div style={{
          width: 64, height: 64, borderRadius: 16,
          background: "var(--primary-softer)", color: "var(--primary)",
          display: "flex", alignItems: "center", justifyContent: "center",
        }}>
          <Icon name="mic" size={26} stroke={1.4} />
        </div>
        <div style={{ fontSize: 14, color: "var(--ink-3)", fontWeight: 500 }}>准备就绪</div>
        <div style={{ fontSize: 12.5, color: "var(--ink-4)", textAlign: "center", maxWidth: 280, lineHeight: 1.6 }}>
          选择麦克风后点击「开始录音」<br />
          每段识别结果会以卡片形式在这里展开
        </div>
      </div>
    );
  }

  if (viewMode === "table") {
    return (
      <div ref={ref} style={{ flex: 1, overflowY: "auto", padding: "0 4px" }}>
        <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 13 }}>
          <thead style={{ position: "sticky", top: 0, background: "var(--bg-app)", zIndex: 1 }}>
            <tr>
              <th style={th}>时间</th>
              <th style={th}>中文</th>
              <th style={th}>English</th>
              <th style={{ ...th, textAlign: "right" }}>操作</th>
            </tr>
          </thead>
          <tbody>
            {segments.map((s) => (
              <SegmentCard key={s.id} seg={s} active={s.id === activeId}
                           onSeek={onSeek} onCopyZh={onCopyZh} onCopyEn={onCopyEn}
                           onCopyBoth={onCopyBoth} onExportOne={onExportOne} viewMode="table" />
            ))}
          </tbody>
        </table>
      </div>
    );
  }

  return (
    <div ref={ref} style={{
      flex: 1,
      overflowY: "auto",
      padding: "8px 4px 12px",
      display: "flex",
      flexDirection: "column",
      gap: viewMode === "card" ? 12 : 18,
    }}>
      {segments.map((s) => (
        <SegmentCard key={s.id} seg={s} active={s.id === activeId}
                     onSeek={onSeek} onCopyZh={onCopyZh} onCopyEn={onCopyEn}
                     onCopyBoth={onCopyBoth} onExportOne={onExportOne} viewMode={viewMode} />
      ))}
    </div>
  );
};

const th = {
  padding: "10px 12px",
  fontSize: 11.5,
  fontWeight: 500,
  color: "var(--ink-3)",
  letterSpacing: "0.04em",
  textTransform: "uppercase",
  borderBottom: "1px solid var(--line)",
  textAlign: "left",
};

window.SegmentList = SegmentList;
