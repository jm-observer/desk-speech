// UI primitives: buttons, switch, modal, dropdown, tooltip, etc.

const Button = ({ kind = "ghost", size = "md", icon, iconRight, children, disabled, onClick, style, title, active }) => {
  const base = {
    display: "inline-flex",
    alignItems: "center",
    justifyContent: "center",
    gap: 7,
    border: "1px solid transparent",
    borderRadius: size === "sm" ? 8 : 10,
    cursor: disabled ? "not-allowed" : "pointer",
    fontWeight: 500,
    transition: "all 140ms ease",
    fontFamily: "var(--font-sans)",
    letterSpacing: "0.01em",
    whiteSpace: "nowrap",
    userSelect: "none",
    opacity: disabled ? 0.45 : 1,
  };
  const sizes = {
    sm: { fontSize: 12.5, padding: "5px 10px", height: 28 },
    md: { fontSize: 13.5, padding: "8px 14px", height: 36 },
    lg: { fontSize: 14, padding: "11px 18px", height: 44 },
  };
  const kinds = {
    primary: { background: "var(--primary)", color: "#fff", borderColor: "var(--primary)", boxShadow: "0 1px 2px rgba(20,80,60,0.15), inset 0 1px 0 rgba(255,255,255,0.15)" },
    danger:  { background: "var(--danger)",  color: "#fff", borderColor: "var(--danger)" },
    soft:    { background: active ? "var(--primary-soft)" : "var(--bg-soft)", color: active ? "var(--primary-deep)" : "var(--ink)", borderColor: active ? "var(--primary-soft)" : "transparent" },
    outline: { background: "var(--bg-card)", color: "var(--ink)", borderColor: "var(--line)" },
    ghost:   { background: "transparent", color: "var(--ink-2)", borderColor: "transparent" },
  };
  return (
    <button
      onClick={disabled ? undefined : onClick}
      disabled={disabled}
      title={title}
      style={{ ...base, ...sizes[size], ...kinds[kind], ...style }}
      onMouseEnter={(e) => {
        if (disabled) return;
        if (kind === "ghost") e.currentTarget.style.background = "var(--bg-soft)";
        if (kind === "outline") e.currentTarget.style.borderColor = "var(--line-strong)";
        if (kind === "soft" && !active) e.currentTarget.style.background = "#eaf0ec";
        if (kind === "primary") e.currentTarget.style.background = "var(--primary-deep)";
      }}
      onMouseLeave={(e) => {
        if (disabled) return;
        if (kind === "ghost") e.currentTarget.style.background = "transparent";
        if (kind === "outline") e.currentTarget.style.borderColor = "var(--line)";
        if (kind === "soft" && !active) e.currentTarget.style.background = "var(--bg-soft)";
        if (kind === "primary") e.currentTarget.style.background = "var(--primary)";
      }}
    >
      {icon && <Icon name={icon} size={size === "sm" ? 13 : 15} />}
      {children}
      {iconRight && <Icon name={iconRight} size={size === "sm" ? 13 : 15} />}
    </button>
  );
};

const IconButton = ({ icon, onClick, title, size = 32, kind = "ghost", disabled, active }) => {
  const styles = {
    ghost: { background: active ? "var(--primary-soft)" : "transparent", color: active ? "var(--primary-deep)" : "var(--ink-2)" },
    soft:  { background: "var(--bg-soft)", color: "var(--ink)" },
  };
  return (
    <button
      onClick={disabled ? undefined : onClick}
      title={title}
      disabled={disabled}
      style={{
        width: size,
        height: size,
        borderRadius: 8,
        border: "none",
        cursor: disabled ? "not-allowed" : "pointer",
        opacity: disabled ? 0.4 : 1,
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
        transition: "all 140ms ease",
        ...styles[kind],
      }}
      onMouseEnter={(e) => { if (!disabled && !active) e.currentTarget.style.background = "var(--bg-soft)"; }}
      onMouseLeave={(e) => { if (!disabled && !active) e.currentTarget.style.background = kind === "soft" ? "var(--bg-soft)" : "transparent"; }}
    >
      <Icon name={icon} size={size > 30 ? 16 : 14} />
    </button>
  );
};

// pill switch
const Switch = ({ checked, onChange, label, sublabel }) => {
  return (
    <label style={{ display: "inline-flex", alignItems: "center", gap: 10, cursor: "pointer", userSelect: "none" }}>
      <span
        onClick={() => onChange(!checked)}
        style={{
          position: "relative",
          width: 34,
          height: 20,
          borderRadius: 999,
          background: checked ? "var(--primary)" : "#d4dad6",
          transition: "background 160ms ease",
          flexShrink: 0,
          boxShadow: "inset 0 1px 2px rgba(0,0,0,0.06)",
        }}
      >
        <span
          style={{
            position: "absolute",
            top: 2,
            left: checked ? 16 : 2,
            width: 16,
            height: 16,
            background: "#fff",
            borderRadius: "50%",
            transition: "left 160ms cubic-bezier(.4,0,.2,1)",
            boxShadow: "0 1px 3px rgba(0,0,0,0.15)",
          }}
        />
      </span>
      {label && (
        <span style={{ display: "flex", flexDirection: "column", lineHeight: 1.25 }}>
          <span style={{ fontSize: 13, color: "var(--ink)", fontWeight: 500 }}>{label}</span>
          {sublabel && <span style={{ fontSize: 11.5, color: "var(--ink-3)" }}>{sublabel}</span>}
        </span>
      )}
    </label>
  );
};

// dropdown (controlled)
const Dropdown = ({ value, options, onChange, disabled, icon, placeholder, width }) => {
  const [open, setOpen] = React.useState(false);
  const ref = React.useRef(null);
  React.useEffect(() => {
    const onDoc = (e) => { if (ref.current && !ref.current.contains(e.target)) setOpen(false); };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, []);
  const current = options.find((o) => o.id === value || o.value === value);
  return (
    <div ref={ref} style={{ position: "relative", width: width || "100%" }}>
      <button
        onClick={() => !disabled && setOpen(!open)}
        disabled={disabled}
        style={{
          width: "100%",
          height: 38,
          padding: "0 12px",
          borderRadius: 10,
          border: `1px solid ${open ? "var(--primary)" : "var(--line)"}`,
          background: disabled ? "var(--bg-soft)" : "var(--bg-card)",
          display: "flex",
          alignItems: "center",
          gap: 8,
          cursor: disabled ? "not-allowed" : "pointer",
          fontSize: 13.5,
          color: "var(--ink)",
          transition: "border-color 140ms ease",
          opacity: disabled ? 0.6 : 1,
          textAlign: "left",
          fontFamily: "inherit",
        }}
      >
        {icon && <Icon name={icon} size={15} style={{ color: "var(--ink-3)", flexShrink: 0 }} />}
        <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          {current ? (current.name || current.label) : <span style={{ color: "var(--ink-4)" }}>{placeholder}</span>}
        </span>
        {current?.note && <span style={{ fontSize: 11.5, color: "var(--ink-4)" }}>{current.note}</span>}
        <Icon name="chevron-down" size={14} style={{ color: "var(--ink-3)", transition: "transform 140ms ease", transform: open ? "rotate(180deg)" : "" }} />
      </button>
      {open && (
        <div
          style={{
            position: "absolute",
            top: "calc(100% + 6px)",
            left: 0,
            right: 0,
            background: "var(--bg-card)",
            border: "1px solid var(--line)",
            borderRadius: 12,
            boxShadow: "var(--shadow-lg)",
            padding: 4,
            zIndex: 50,
            maxHeight: 280,
            overflowY: "auto",
            animation: "fadeUp 140ms ease",
          }}
        >
          {options.map((o) => {
            const id = o.id ?? o.value;
            const sel = id === value;
            return (
              <button
                key={id}
                onClick={() => { onChange(id); setOpen(false); }}
                style={{
                  width: "100%",
                  display: "flex",
                  alignItems: "center",
                  gap: 8,
                  padding: "8px 10px",
                  borderRadius: 8,
                  border: "none",
                  background: sel ? "var(--primary-soft)" : "transparent",
                  color: sel ? "var(--primary-deep)" : "var(--ink)",
                  cursor: "pointer",
                  fontSize: 13.5,
                  textAlign: "left",
                  fontFamily: "inherit",
                  transition: "background 100ms ease",
                }}
                onMouseEnter={(e) => { if (!sel) e.currentTarget.style.background = "var(--bg-soft)"; }}
                onMouseLeave={(e) => { if (!sel) e.currentTarget.style.background = "transparent"; }}
              >
                <span style={{ flex: 1 }}>{o.name || o.label}</span>
                {o.note && <span style={{ fontSize: 11.5, color: "var(--ink-4)" }}>{o.note}</span>}
                {sel && <Icon name="check" size={14} />}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
};

// Modal shell
const Modal = ({ open, onClose, title, subtitle, icon, children, width = 560, footer }) => {
  React.useEffect(() => {
    if (!open) return;
    const onKey = (e) => { if (e.key === "Escape") onClose && onClose(); };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, onClose]);
  if (!open) return null;
  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        background: "rgba(20, 30, 25, 0.32)",
        backdropFilter: "blur(6px)",
        WebkitBackdropFilter: "blur(6px)",
        zIndex: 100,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        animation: "fadeUp 160ms ease",
      }}
      onClick={onClose}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        style={{
          width,
          maxWidth: "92vw",
          maxHeight: "88vh",
          background: "var(--bg-card)",
          borderRadius: 18,
          boxShadow: "var(--shadow-lg), 0 0 0 1px rgba(20,40,32,0.04)",
          display: "flex",
          flexDirection: "column",
          overflow: "hidden",
          animation: "fadeUp 200ms cubic-bezier(.4,0,.2,1)",
        }}
      >
        <div style={{ display: "flex", alignItems: "flex-start", padding: "20px 22px 12px", gap: 12 }}>
          {icon && (
            <div style={{
              width: 36,
              height: 36,
              borderRadius: 10,
              background: "var(--primary-soft)",
              color: "var(--primary-deep)",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              flexShrink: 0,
            }}>
              <Icon name={icon} size={18} />
            </div>
          )}
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontSize: 16, fontWeight: 600, color: "var(--ink)" }}>{title}</div>
            {subtitle && <div style={{ fontSize: 12.5, color: "var(--ink-3)", marginTop: 2 }}>{subtitle}</div>}
          </div>
          <IconButton icon="close" onClick={onClose} title="关闭" />
        </div>
        <div style={{ flex: 1, overflowY: "auto", padding: "8px 22px 22px" }}>
          {children}
        </div>
        {footer && (
          <div style={{
            padding: "14px 22px",
            borderTop: "1px solid var(--line)",
            background: "var(--bg-softer)",
            display: "flex",
            justifyContent: "flex-end",
            gap: 8,
          }}>
            {footer}
          </div>
        )}
      </div>
    </div>
  );
};

// Field label + input wrapper
const Field = ({ label, hint, children, span = 1 }) => (
  <div style={{ display: "flex", flexDirection: "column", gap: 6, gridColumn: `span ${span}` }}>
    <label style={{ fontSize: 12, fontWeight: 500, color: "var(--ink-2)", letterSpacing: "0.01em" }}>{label}</label>
    {children}
    {hint && <div style={{ fontSize: 11.5, color: "var(--ink-4)", marginTop: 1 }}>{hint}</div>}
  </div>
);

const TextInput = ({ value, onChange, placeholder, type = "text", suffix, monospace, ...rest }) => (
  <div style={{ position: "relative", display: "flex", alignItems: "center" }}>
    <input
      type={type}
      value={value}
      onChange={(e) => onChange(type === "number" ? e.target.value : e.target.value)}
      placeholder={placeholder}
      {...rest}
      style={{
        width: "100%",
        height: 36,
        padding: suffix ? "0 44px 0 12px" : "0 12px",
        borderRadius: 9,
        border: "1px solid var(--line)",
        background: "var(--bg-card)",
        fontSize: 13.5,
        outline: "none",
        fontFamily: monospace ? "var(--font-mono)" : "inherit",
        transition: "border-color 140ms ease, box-shadow 140ms ease",
      }}
      onFocus={(e) => { e.target.style.borderColor = "var(--primary)"; e.target.style.boxShadow = "0 0 0 3px var(--primary-softer)"; }}
      onBlur={(e) => { e.target.style.borderColor = "var(--line)"; e.target.style.boxShadow = "none"; }}
    />
    {suffix && (
      <span style={{ position: "absolute", right: 12, fontSize: 11.5, color: "var(--ink-4)", fontFamily: "var(--font-mono)" }}>
        {suffix}
      </span>
    )}
  </div>
);

const Slider = ({ value, min, max, step = 0.01, onChange, suffix }) => (
  <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
    <input
      type="range"
      min={min}
      max={max}
      step={step}
      value={value}
      onChange={(e) => onChange(parseFloat(e.target.value))}
      style={{
        flex: 1,
        accentColor: "var(--primary)",
        height: 4,
      }}
    />
    <span style={{ fontFamily: "var(--font-mono)", fontSize: 12, color: "var(--ink-2)", minWidth: 60, textAlign: "right" }}>
      {value}{suffix || ""}
    </span>
  </div>
);

const Tag = ({ children, color = "neutral" }) => {
  const palette = {
    neutral: { bg: "var(--bg-soft)", fg: "var(--ink-2)" },
    mint:    { bg: "var(--primary-soft)", fg: "var(--primary-deep)" },
    warn:    { bg: "#fef3e0", fg: "var(--warning)" },
    danger:  { bg: "var(--danger-soft)", fg: "var(--danger)" },
  };
  const p = palette[color];
  return (
    <span style={{
      display: "inline-flex",
      alignItems: "center",
      gap: 4,
      padding: "2px 8px",
      borderRadius: 999,
      background: p.bg,
      color: p.fg,
      fontSize: 11.5,
      fontWeight: 500,
      letterSpacing: "0.01em",
    }}>
      {children}
    </span>
  );
};

Object.assign(window, { Button, IconButton, Switch, Dropdown, Modal, Field, TextInput, Slider, Tag });
