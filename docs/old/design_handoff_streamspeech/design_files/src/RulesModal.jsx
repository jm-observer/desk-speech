// Word correction rules modal
const RulesModal = ({ open, onClose, rules, onChange }) => {
  const [draft, setDraft] = React.useState(rules);
  const [adding, setAdding] = React.useState(false);
  const [newRule, setNewRule] = React.useState({ src: "", dst: "", priority: 50 });
  React.useEffect(() => { if (open) { setDraft(rules); setAdding(false); setNewRule({ src: "", dst: "", priority: 50 }); } }, [open, rules]);

  const update = (id, patch) => {
    setDraft((d) => d.map((r) => r.id === id ? { ...r, ...patch } : r));
  };
  const remove = (id) => setDraft((d) => d.filter((r) => r.id !== id));
  const submitNew = () => {
    if (!newRule.src.trim()) return;
    const id = Math.max(0, ...draft.map((r) => r.id)) + 1;
    setDraft((d) => [...d, { id, enabled: true, ...newRule, priority: parseInt(newRule.priority) || 50 }]);
    setAdding(false);
    setNewRule({ src: "", dst: "", priority: 50 });
  };

  const save = () => { onChange(draft); onClose(); };

  return (
    <Modal
      open={open}
      onClose={onClose}
      title="词修正规则"
      subtitle="对识别文本应用词级替换，规则按优先级从小到大执行"
      icon="wand"
      width={680}
      footer={
        <>
          <Button kind="ghost" onClick={onClose}>取消</Button>
          <Button kind="primary" icon="check" onClick={save}>保存并重载</Button>
        </>
      }
    >
      {/* Header row */}
      <div style={{
        display: "grid",
        gridTemplateColumns: "44px 1fr 1fr 88px 56px",
        gap: 8,
        padding: "8px 10px",
        fontSize: 11,
        fontWeight: 500,
        color: "var(--ink-3)",
        letterSpacing: "0.04em",
        textTransform: "uppercase",
        borderBottom: "1px solid var(--line)",
      }}>
        <div>启用</div>
        <div>源词</div>
        <div>替换为</div>
        <div>优先级</div>
        <div></div>
      </div>

      {/* Rule rows */}
      <div style={{ display: "flex", flexDirection: "column" }}>
        {draft.length === 0 && (
          <div style={{ padding: "32px 0", textAlign: "center", color: "var(--ink-4)", fontSize: 13 }}>
            暂无规则，点击下方「新增」开始。
          </div>
        )}
        {draft.map((r) => (
          <div key={r.id} style={{
            display: "grid",
            gridTemplateColumns: "44px 1fr 1fr 88px 56px",
            gap: 8,
            alignItems: "center",
            padding: "8px 10px",
            borderBottom: "1px solid var(--line)",
            transition: "background 120ms ease",
          }}
            onMouseEnter={(e) => e.currentTarget.style.background = "var(--bg-softer)"}
            onMouseLeave={(e) => e.currentTarget.style.background = "transparent"}
          >
            <Switch checked={r.enabled} onChange={(v) => update(r.id, { enabled: v })} />
            <input
              value={r.src}
              onChange={(e) => update(r.id, { src: e.target.value })}
              style={ruleInput(!r.src.trim())}
            />
            <input
              value={r.dst}
              onChange={(e) => update(r.id, { dst: e.target.value })}
              style={ruleInput(false)}
              placeholder="(留空则删除)"
            />
            <input
              type="number"
              value={r.priority}
              onChange={(e) => update(r.id, { priority: parseInt(e.target.value) || 0 })}
              style={{ ...ruleInput(false), fontFamily: "var(--font-mono)", textAlign: "right" }}
            />
            <IconButton icon="trash" onClick={() => remove(r.id)} title="删除" />
          </div>
        ))}
      </div>

      {/* Add row */}
      {adding ? (
        <div style={{
          display: "grid",
          gridTemplateColumns: "44px 1fr 1fr 88px 56px",
          gap: 8,
          alignItems: "center",
          padding: "10px",
          background: "var(--primary-softer)",
          borderRadius: 10,
          marginTop: 12,
        }}>
          <Switch checked={true} onChange={() => {}} />
          <input
            placeholder="输入源词"
            value={newRule.src}
            autoFocus
            onChange={(e) => setNewRule({ ...newRule, src: e.target.value })}
            onKeyDown={(e) => e.key === "Enter" && submitNew()}
            style={ruleInput(false)}
          />
          <input
            placeholder="替换为"
            value={newRule.dst}
            onChange={(e) => setNewRule({ ...newRule, dst: e.target.value })}
            onKeyDown={(e) => e.key === "Enter" && submitNew()}
            style={ruleInput(false)}
          />
          <input
            type="number"
            value={newRule.priority}
            onChange={(e) => setNewRule({ ...newRule, priority: e.target.value })}
            style={{ ...ruleInput(false), fontFamily: "var(--font-mono)", textAlign: "right" }}
          />
          <Button kind="primary" size="sm" onClick={submitNew} style={{ height: 32 }}>添加</Button>
        </div>
      ) : (
        <div style={{ paddingTop: 14 }}>
          <Button kind="soft" icon="plus" onClick={() => setAdding(true)}>新增规则</Button>
        </div>
      )}
    </Modal>
  );
};

const ruleInput = (invalid) => ({
  height: 32,
  padding: "0 10px",
  borderRadius: 7,
  border: `1px solid ${invalid ? "var(--danger)" : "var(--line)"}`,
  background: "var(--bg-card)",
  fontSize: 13,
  fontFamily: "inherit",
  outline: "none",
  width: "100%",
});

window.RulesModal = RulesModal;
