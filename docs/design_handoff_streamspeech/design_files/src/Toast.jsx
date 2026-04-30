// Toast / status flash
const ToastContext = React.createContext(null);
const ToastProvider = ({ children }) => {
  const [toasts, setToasts] = React.useState([]);
  const push = React.useCallback((msg, kind = "ok") => {
    const id = Math.random().toString(36).slice(2);
    setToasts((t) => [...t, { id, msg, kind }]);
    setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), 2200);
  }, []);
  return (
    <ToastContext.Provider value={push}>
      {children}
      <div style={{
        position: "fixed",
        bottom: 24,
        left: "50%",
        transform: "translateX(-50%)",
        display: "flex",
        flexDirection: "column",
        gap: 8,
        zIndex: 200,
        pointerEvents: "none",
      }}>
        {toasts.map((t) => {
          const palette = {
            ok:   { bg: "rgba(20,30,25,0.92)", icon: "check", fg: "#dcebd8" },
            err:  { bg: "rgba(180,60,50,0.95)", icon: "alert", fg: "#fff" },
            info: { bg: "rgba(20,30,25,0.92)", icon: "info", fg: "#fff" },
          };
          const p = palette[t.kind] || palette.ok;
          return (
            <div
              key={t.id}
              style={{
                display: "flex",
                alignItems: "center",
                gap: 8,
                padding: "10px 14px",
                background: p.bg,
                color: "#fff",
                borderRadius: 12,
                boxShadow: "var(--shadow-lg)",
                fontSize: 13,
                animation: "fadeUp 180ms ease",
                backdropFilter: "blur(8px)",
              }}
            >
              <Icon name={p.icon} size={15} style={{ color: p.fg }} />
              {t.msg}
            </div>
          );
        })}
      </div>
    </ToastContext.Provider>
  );
};
const useToast = () => React.useContext(ToastContext);

window.ToastProvider = ToastProvider;
window.useToast = useToast;
