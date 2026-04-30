// Mock audio player — scrubbable, plays a fake waveform.
const AudioPlayer = ({ duration = 0, current, playing, onSeek, onTogglePlay, disabled, segments = [] }) => {
  const trackRef = React.useRef(null);
  const onMouseDown = (e) => {
    if (!trackRef.current || disabled) return;
    const rect = trackRef.current.getBoundingClientRect();
    const pct = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
    onSeek && onSeek(pct * duration);
    const move = (ev) => {
      const r2 = trackRef.current.getBoundingClientRect();
      const p = Math.max(0, Math.min(1, (ev.clientX - r2.left) / r2.width));
      onSeek && onSeek(p * duration);
    };
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  const pct = duration > 0 ? (current / duration) * 100 : 0;

  // Render fake static waveform peaks based on the duration
  const peaks = React.useMemo(() => {
    if (duration <= 0) return [];
    const N = 120;
    const arr = [];
    for (let i = 0; i < N; i++) {
      // deterministic-ish noise
      const a = Math.sin(i * 0.37) * 0.5 + Math.sin(i * 1.13) * 0.3 + Math.sin(i * 2.5) * 0.2;
      arr.push(Math.max(0.12, Math.min(1, Math.abs(a) + 0.18)));
    }
    return arr;
  }, [duration]);

  // segment markers
  const markers = segments.map((s) => ({ start: s.start / duration * 100, end: s.end / duration * 100 }));

  return (
    <div style={{
      display: "flex",
      alignItems: "center",
      gap: 14,
      padding: "14px 16px",
      background: "var(--bg-card)",
      border: "1px solid var(--line)",
      borderRadius: 14,
      opacity: disabled ? 0.55 : 1,
    }}>
      <button
        onClick={() => !disabled && onTogglePlay && onTogglePlay()}
        disabled={disabled}
        style={{
          width: 38,
          height: 38,
          borderRadius: "50%",
          border: "none",
          background: "var(--primary)",
          color: "#fff",
          cursor: disabled ? "not-allowed" : "pointer",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          flexShrink: 0,
          boxShadow: "0 2px 8px rgba(20, 161, 129, 0.25)",
          transition: "transform 120ms ease",
        }}
        onMouseEnter={(e) => !disabled && (e.currentTarget.style.transform = "scale(1.05)")}
        onMouseLeave={(e) => (e.currentTarget.style.transform = "scale(1)")}
      >
        <Icon name={playing ? "pause" : "play"} size={16} />
      </button>

      <div style={{ flex: 1, minWidth: 0 }}>
        <div
          ref={trackRef}
          onMouseDown={onMouseDown}
          style={{
            position: "relative",
            height: 36,
            cursor: disabled ? "default" : "pointer",
            display: "flex",
            alignItems: "center",
            gap: 1.5,
          }}
        >
          {peaks.map((p, i) => {
            const barPct = (i / peaks.length) * 100;
            const before = barPct < pct;
            return (
              <div key={i} style={{
                flex: 1,
                height: `${p * 28}px`,
                minHeight: 4,
                borderRadius: 999,
                background: before ? "var(--primary)" : "var(--line-strong)",
                opacity: before ? 0.95 : 0.7,
                transition: "background 120ms ease",
              }} />
            );
          })}
          {/* segment dividers */}
          {markers.map((m, i) => (
            <div key={i} style={{
              position: "absolute",
              left: `${m.end}%`,
              top: 4,
              bottom: 4,
              width: 1,
              background: "rgba(20,40,32,0.08)",
            }} />
          ))}
          {/* playhead */}
          <div style={{
            position: "absolute",
            left: `calc(${pct}% - 2px)`,
            top: -2,
            bottom: -2,
            width: 4,
            background: "var(--primary-deep)",
            borderRadius: 2,
            boxShadow: "0 0 0 3px rgba(20,161,129,0.18)",
          }} />
        </div>
      </div>

      <div style={{ fontFamily: "var(--font-mono)", fontSize: 12, color: "var(--ink-3)", minWidth: 92, textAlign: "right" }}>
        {fmtTime(current)} <span style={{ color: "var(--ink-4)" }}>/ {fmtTime(duration)}</span>
      </div>
    </div>
  );
};

window.AudioPlayer = AudioPlayer;
