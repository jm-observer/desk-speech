// Mock segment script — used to drive the simulated recording flow.
const MOCK_SEGMENTS = [
  {
    raw: "好今天我们简单过一下这周的进度然后看看下周要重点推进哪些事情",
    polished: "好，今天我们简单过一下本周进度，然后看看下周要重点推进哪些事情。",
    en: "Alright, let's quickly review this week's progress and decide which items to push next week.",
    duration: 6.4,
  },
  {
    raw: "首先模型这边小波那个VAD的阈值我又调了一版默认零点四八",
    polished: "首先在模型这边，小波把 VAD 的阈值又调了一版，默认改到 0.48。",
    en: "First on the model side, Xiaobo tuned the VAD threshold again — default is now 0.48.",
    duration: 5.1,
  },
  {
    raw: "实测下来误触发率下来不少嗯但是短句尾巴有时候会被切掉",
    polished: "实测下来误触发率降了不少。不过短句尾巴有时候会被切掉。",
    en: "False triggers are down noticeably, but short sentence tails sometimes get clipped.",
    duration: 4.7,
  },
  {
    raw: "所以下周计划是把最小语音时长往下再压一点然后再跑一轮回归",
    polished: "所以下周计划是把最小语音时长再压低一点，然后再跑一轮回归测试。",
    en: "So next week we'll lower the minimum speech duration a bit and run another regression pass.",
    duration: 5.8,
  },
  {
    raw: "另外LLM那边换成claude haiku之后润色速度快了大概三倍但是费用也会涨",
    polished: "另外 LLM 这边换成 Claude Haiku 后，润色速度快了大约三倍，但费用也会上涨。",
    en: "Also after switching the LLM to Claude Haiku, polishing is about 3× faster, but costs go up.",
    duration: 6.2,
  },
  {
    raw: "我们下周可以做一个开关让用户选要速度还是要质量",
    polished: "我们下周可以加一个开关，让用户在速度和质量之间自由选择。",
    en: "Next week we can add a toggle that lets users pick between speed and quality.",
    duration: 4.4,
  },
];

const MOCK_DEVICES = [
  { id: "default", name: "MacBook Pro 麦克风", note: "默认输入" },
  { id: "airpods", name: "AirPods Pro", note: "蓝牙" },
  { id: "scarlett", name: "Focusrite Scarlett 2i2", note: "USB · 48kHz" },
  { id: "shure",   name: "Shure MV7+", note: "USB · 主播话筒" },
];

const MOCK_RULES = [
  { id: 1, enabled: true,  src: "微博",     dst: "小波",   priority: 10 },
  { id: 2, enabled: true,  src: "VID",      dst: "VAD",    priority: 20 },
  { id: 3, enabled: true,  src: "Cloud",    dst: "Claude", priority: 30 },
  { id: 4, enabled: false, src: "回归测试", dst: "回归",   priority: 40 },
  { id: 5, enabled: true,  src: "麦考",     dst: "Mic",    priority: 50 },
];

const MOCK_MODELS = [
  "claude-haiku-4-5",
  "claude-sonnet-4-5",
  "gpt-4o-mini",
  "qwen2.5-7b-instruct",
  "deepseek-v3",
];

// initial segments shown on load — last finished session
const INITIAL_SEGMENTS = [
  {
    id: "s0",
    start: 0,
    end: 5.2,
    raw: "上周五的对话已经识别完毕",
    polished: "上周五的对话已经识别完毕。",
    en: "Last Friday's conversation has been transcribed.",
    status: "done",
  },
  {
    id: "s1",
    start: 5.2,
    end: 12.6,
    raw: "你可以点击开始录音继续新的会话或回放历史音频",
    polished: "你可以点击「开始录音」继续新的会话，或回放历史音频。",
    en: "You can click \"Start recording\" to begin a new session, or replay the previous audio.",
    status: "done",
  },
];

// helpers
const fmtTime = (s) => {
  if (s == null || isNaN(s)) return "00:00";
  const m = Math.floor(s / 60);
  const sec = Math.floor(s % 60);
  return `${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}`;
};
const fmtTimeMs = (s) => {
  if (s == null || isNaN(s)) return "00:00.0";
  const m = Math.floor(s / 60);
  const sec = (s % 60).toFixed(1);
  return `${String(m).padStart(2, "0")}:${sec.padStart(4, "0")}`;
};

window.MOCK_SEGMENTS = MOCK_SEGMENTS;
window.MOCK_DEVICES = MOCK_DEVICES;
window.MOCK_RULES = MOCK_RULES;
window.MOCK_MODELS = MOCK_MODELS;
window.INITIAL_SEGMENTS = INITIAL_SEGMENTS;
window.fmtTime = fmtTime;
window.fmtTimeMs = fmtTimeMs;
