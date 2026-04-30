// Realtime waveform visualization (mock — driven by recording state)
const Waveform = ({ active, intensity = 0.5, style }) => {
  // 40 bars, animated heights
  const bars = 44;
  const [tick, setTick] = React.useState(0);
  React.useEffect(() => {
    if (!active) return;
    let raf;
    const loop = () => {
      setTick((t) => t + 1);
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(raf);
  }, [active]);

  const heights = React.useMemo(() => {
    return Array.from({ length: bars }, (_, i) => {
      if (!active) return 0.08;
      // Pseudo-random but smooth using sine layered with noise
      const t = Date.now() / 1000;
      const phase = (i / bars) * Math.PI * 4;
      const wave1 = Math.sin(t * 6 + phase) * 0.5 + 0.5;
      const wave2 = Math.sin(t * 11 + phase * 2.3) * 0.5 + 0.5;
      const wave3 = Math.sin(t * 3.5 + phase * 0.7) * 0.5 + 0.5;
      const v = (wave1 * 0.5 + wave2 * 0.3 + wave3 * 0.2) * intensity;
      return Math.max(0.06, Math.min(1, v + 0.1));
    });
    // eslint-disable-next-line
  }, [tick, active, intensity]);

  return (
    <div style={{
      display: "flex",
      alignItems: "center",
      justifyContent: "center",
      gap: 3,
      height: 64,
      width: "100%",
      ...style,
    }}>
      {heights.map((h, i) => {
        const dist = Math.abs(i - bars / 2) / (bars / 2); // 0 center .. 1 edge
        const opacity = active ? (1 - dist * 0.4) : 0.4;
        return (
          <div
            key={i}
            style={{
              width: 3,
              height: `${Math.max(6, h * 56)}px`,
              borderRadius: 999,
              background: active
                ? `linear-gradient(180deg, var(--primary), var(--primary-deep))`
                : "var(--line-strong)",
              opacity,
              transition: "height 70ms ease-out",
            }}
          />
        );
      })}
    </div>
  );
};

window.Waveform = Waveform;
