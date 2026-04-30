# Handoff: StreamSpeech — Offline Streaming ASR Desktop App

## Overview

StreamSpeech is an offline, streaming voice-recognition desktop app (intended target: Tauri / Electron). The user records from a system microphone, the app produces realtime Chinese transcription segments via a local ASR pipeline (VAD → ASR → LLM polishing → optional EN translation), and exposes copy / export / playback affordances.

This handoff bundles the UI design as an HTML prototype plus this written spec. It covers the **full visual + interaction design** of the main window: control rail, segment list, audio scrubber, settings modal, word-correction modal, and the simple-mode floating widget.

## About the Design Files

The files in `design_files/` are **design references created in HTML/JSX** — a clickable prototype demonstrating intended look, layout, copy, and behavior. They are **not production code to copy directly**.

Your task is to **recreate these designs in the target codebase's existing environment** using its established patterns and libraries. If the project is greenfield, choose the most appropriate framework (Tauri + React/Svelte, or Electron + React, are natural fits given the desktop target). Do not ship the prototype HTML as the actual app — the prototype mocks all data, audio, and ASR behavior in JS.

## Fidelity

**High-fidelity (hifi).** Final colors, typography, spacing, shadows, motion, and copy are all locked in. Implement pixel-perfectly using the codebase's component library — replace the prototype's hand-rolled primitives (`Button`, `Switch`, `Dropdown`, `Modal`) with the equivalents from the host design system, but match dimensions / colors / behavior exactly.

---

## Application Structure

The app is a single-window desktop application with two top-level UI modes:

| Mode | Layout | Use |
|---|---|---|
| **Detailed** (default) | Two-column layout: 320px control rail + flex-1 results panel | Normal usage |
| **Simple** | A single 320px-wide floating card anchored top-left of the window (transparent surrounding background) | Always-on-top mini widget for quick recording without the results panel taking screen space |

A small pill button in the control panel (top-right area) toggles between the two modes. In a real Tauri/Electron build, **simple mode should additionally resize the OS window** to roughly 352 × 220px and enable always-on-top; detailed mode restores normal window size.

---

## Screens & Views

### 1. Detailed mode — Main window

Two-column flex layout, fills the full window.

#### 1.1 Left control rail (320px wide)

Vertical stack, padding `26px 24px`, gap `22px`, background `--bg-app`, right border `1px solid --line`.

Contents top-to-bottom:

1. **App header row** — 32×32 gradient logo tile (mint→deep-mint, border-radius 9px, soft drop shadow), app name "StreamSpeech" (14.5px / 600), subtitle "离线语音识别 · v1.4.0" (11px / `--ink-4`), and the mode-switch pill on the right.
2. **Status chip** — pill, see [Status chip](#status-chip).
3. **Input device dropdown** — labeled "输入设备" (uppercase 11.5px label `--ink-3`). Custom dropdown with mic-device icon. Disabled while recording.
4. **Record card** — see [Record card](#record-card).
5. **Switches block** — two `Switch` rows:
   - "自动复制到剪贴板" / sub: "新分段识别完成后自动写入"
   - "显示英文翻译" / sub: "LLM 同步生成对照翻译"
6. **Spacer** (`flex: 1`).
7. **Footer button grid** — 2-column grid, 8px gap, top border `1px --line`, padding-top 16px:
   - `[清空结果]` `[词修正]`
   - `[识别参数设置]` (full-width, span 2)

   All three are `kind="soft"` style buttons. "清空结果" and "识别参数设置" disabled while recording.

#### 1.2 Right results panel (flex 1)

Vertical stack, fills remaining width.

1. **Toolbar row** — horizontal flex, padding `14px 24px`, bottom border `1px --line`. Contains outline-style action buttons:
   - `[复制中文]` `[复制英文]` `[含时间戳]` `[导出 SRT]` `[保存音频]`
   - All disabled until at least one segment has finished. Right-aligned hint text "开始录音后将启用导出" while empty.
2. **Segment list** — flex 1, scrollable, padding `14px 24px`, gap between cards 12px. See [Segment card](#segment-card).
3. **Audio player** — sticky bottom strip, padding `0 24px 22px`, only visible when `totalDuration > 0`. See [Audio player](#audio-player).

### 2. Simple mode — Floating mini widget

A single 320px-wide rounded card (`border-radius: 18px`, `border: 1px --line`, `box-shadow: --shadow-lg`) anchored at top-left with 16px margin from the window edges. Surrounding window background is transparent (so the OS window chrome / desktop shows through if frameless).

Contents (padding 14px, gap 12px):

1. **Top bar** — 22×22 logo tile + "StreamSpeech" name (12.5px / 600) + flex spacer + status chip + mode-switch pill.
2. **Compact record card** — same gradient background as detailed-mode record card but tighter:
   - `padding: 14px 14px 12px`, `border-radius: 14px`
   - 44px-tall waveform
   - 24px monospace timer (with pulsing red dot when recording)
   - Single full-width primary button (38px tall, 10px radius), label "开始录音" / "停止录音"

No device dropdown, settings, or other affordances in simple mode — recording uses the most recently selected device + settings.

### 3. Settings modal — "识别参数设置"

Centered modal, max-width ~720px, `--bg-card`, `border-radius: --radius-xl`, `--shadow-lg`, scrim `rgba(20,40,32,0.42)` with backdrop-blur(2px). Three tabs: **VAD**, **ASR**, **LLM 润色**.

- **Tab strip** at top — pill style, active tab gets `--primary-soft` background + `--primary-deep` text.
- **Form layout** — 2-column grid (`repeat(2, 1fr)`, 16px gap), fields stack to 1 column at narrow widths.
- **Form fields**: labeled (11.5px uppercase `--ink-3`), with sub-help text (11px `--ink-4`). Inputs use the same primitives as the rest of the app (number input, slider, dropdown, multiline text).
- **Footer** — right-aligned `[取消]` `[应用并重载模型]`. Applying triggers the "模型加载中" state (~1.4s) then a toast "模型已就绪".

VAD fields: 静音阈值 (ms slider 200–2000), 端点容差 (ms), 最小语音段时长 (ms), 灵敏度 (slider 0–1).
ASR fields: 模型 (dropdown: `paraformer-zh-streaming`, `whisper-large-v3-zh`, `sensevoice-small`), 语言, 最大候选数, 实时模式 toggle.
LLM fields: 模型 (dropdown), 系统提示 (textarea, 6 rows), 温度 (slider 0–1), 启用润色 toggle, 同步翻译为英文 toggle.

### 4. Word correction modal — "词修正"

Same modal chrome. Body is a list of rule rows:

- Each row: `[原词输入] → [替换为输入] [×]`
- "+ 添加规则" outline button below the list
- Footer: `[取消]` `[保存 N 条规则]`

Rules persist between sessions (in real impl, store in app data dir).

---

## Reusable Components

### Status chip

Pill, `padding: 5px 11px`, `border-radius: 999px`, font `12px / 500`. Leading 7×7 dot. Six states:

| State | Label | Background | Text | Dot |
|---|---|---|---|---|
| `idle` | 就绪 | `--bg-soft` | `--ink-2` | `#7a857f` |
| `initializing` | 模型加载中 | `#fef4e2` | `--warning` | `#c98a2b` |
| `recording` | 正在录音 | `--primary-soft` | `--primary-deep` | `--primary` (pulsing) |
| `processing` | 处理中 | `#fef4e2` | `--warning` | `#c98a2b` |
| `error` | 异常 | `--danger-soft` | `--danger` | `--danger` |
| `finished` | 已完成 | `--primary-soft` | `--primary-deep` | `--primary` |

`pulseDot` keyframe: scale 1→1.6, opacity 1→0, 1.4s ease-in-out infinite.

### Record card

The hero element of the control rail. Vertical flex, items centered.

- Gradient background: `linear-gradient(180deg, --primary-softer 0%, transparent 100%)`
- `border: 1px --primary-soft`, `border-radius: 18px`, padding `20px 16px 22px`, gap 16px
- **Waveform** — 64px tall, see [Waveform](#waveform). Shows idle (low-amplitude) when not recording, active (high-amplitude) when recording.
- **Timer** — `Geist Mono` 30px / 500, `letter-spacing: 0.02em`, `font-variant-numeric: tabular-nums`, format `MM:SS.cc` (centiseconds). Color `--primary-deep` while recording, `--ink-3` otherwise. Leading 9px red pulsing dot only while recording.
- **Action button** — full width, 46px tall, 12px radius:
  - Idle: gradient mint primary (`linear-gradient(180deg, --primary 0%, --primary-deep 100%)`), white text, soft shadow, label "开始录音" with mic icon. Hover lifts -1px and deepens shadow. Disabled (cdd5cf) until model is ready and a device is selected.
  - Recording: outline danger style (`--bg-card` bg, `--danger` border + text), label "停止录音" with stop icon. Hover background `--danger-soft`.
- **Hotkey hint** — 11px `--ink-4`, "快捷键 ⌘ R", with `kbd` chips.

### Segment card

One per finished or in-flight ASR segment. Card style:

- `--bg-card`, `border: 1px --line`, `border-radius: --radius-lg`, padding 14px 16px, `box-shadow: --shadow-sm`
- Hover lifts shadow to `--shadow-md` and shifts border to `--line-strong`
- Active (currently playing) segment: `border: 1px --primary`, `box-shadow: 0 0 0 3px --primary-soft`

Card layout (vertical flex, gap 10):

1. **Header row** (horizontal):
   - **Time pill** — clickable, `--bg-soft` bg, mono 11px, format `[00:12.4 → 00:18.9]`. Hover `--primary-soft`. Click seeks audio player.
   - **Duration** — small `--ink-4` text, `6.5s`
   - Spacer
   - **Per-card actions** — 3 small icon buttons (28×28, hover `--bg-soft`):
     - Copy 中文 (with checkmark feedback for 1.2s after click)
     - Copy 英文 (only if EN exists)
     - Export this segment as WAV
2. **Chinese text** — `--ink`, 15px / 1.7 line-height, `text-wrap: pretty`. The polished text. If polished differs from raw, a small "查看原文" disclosure toggles a dim `--ink-3` blockquote with the raw.
3. **English text** (if `showEnglish` and `seg.en` exists) — `--ink-2`, 14px / 1.7, slight margin-top. Italic Geist would be acceptable but Noto Sans SC stays the default.

In-flight (status `processing`) segment: shimmer placeholder animation on the text lines, status chip "处理中" replaces actions.

Card entry animation: `fadeUp` 220ms.

### Waveform

Canvas-driven bar visualizer. ~32 bars, 3px wide, 2px gap, rounded caps. Color `--primary` at full opacity for active bars; `--ink-4` at 30% opacity for idle. Two driving inputs:

- `active: boolean` — when true, animate bar amplitudes via Perlin-noise-ish pseudo-random walk biased high.
- `intensity: 0..1` — global amplitude multiplier (the simple/detailed cards differ here).

### Audio player

Sticky bottom strip. Visible only after first finished segment.

- Layout: play/pause button (32×32, gradient primary, white triangle/square icon) + scrubber + time display.
- **Scrubber** — full-width track, 4px tall, `--bg-soft` background, fill in `--primary`. Click anywhere to seek. Each segment's range is marked with subtle ticks on the track at its start time.
- **Time** — `Geist Mono` 12px, format `00:23 / 01:42`, `--ink-3`.
- Disabled while recording.

### Buttons

Three kinds:

| Kind | Background | Border | Text | Hover |
|---|---|---|---|---|
| `primary` (record start only) | gradient mint | none | white | shadow grows |
| `outline` | `--bg-card` | `1px --line` | `--ink-2` | bg `--bg-soft`, border `--line-strong` |
| `soft` | `--bg-soft` | none | `--ink-2` | bg `--primary-soft`, text `--primary-deep` |

Sizes: `sm` (28px tall, 12px font), default (34px tall, 13px font), `lg` (40px). Border-radius 8 / 10 / 12.

### Switch

Custom row: label + sub-label on left, iOS-style toggle (40×24 track, 20px thumb, `--primary` when on, `--line-strong` when off, 200ms transition) on right.

### Dropdown

`--bg-card`, `1px --line`, 10px radius, 38px tall, padding 0 14px, leading optional icon, trailing chevron icon. Open menu: `--shadow-md`, max-height 280px scroll. Active option `--primary-soft` bg.

### Modal

Scrim + centered card. Card has a header row (title + close × icon button), scrollable body, sticky footer. ESC closes. Click scrim closes (unless dirty + has unsaved changes — prompt).

### Toast

Bottom-center stack, 12px from edge. Pill `--bg-card`, `1px --line`, `--shadow-md`, padding `10px 16px`, leading icon (success / info / error). Auto-dismiss 2.6s. fadeUp entry, fade-out exit.

### Tweaks panel (design-time only)

The bundled `tweaks-panel.jsx` is a starter component used by the prototype to expose live design knobs (primary hue slider, density, radius, viewMode, uiMode). **Do not ship** — strip it from the production build.

---

## Interactions & Behavior

### Recording flow

1. App boots → status `initializing` for ~600ms while model loads → status `idle`.
2. User selects input device (auto-pre-selects system default).
3. User clicks **开始录音** (or presses ⌘R):
   - Status → `recording`. Timer starts. Waveform amplifies.
   - Microphone capture begins. Audio streams to VAD → ASR → LLM pipeline.
4. As VAD detects speech-end boundaries, in-flight segment cards appear in the results panel with a shimmer placeholder. When ASR + polishing return, the card fills in with text. If `showEnglish` is on, the EN translation streams in shortly after.
5. If `autoCopy` is on, finished segments append to the system clipboard automatically — show a toast "已复制".
6. User clicks **停止录音** (or ⌘R again):
   - Status → `processing` while pending segments drain → `finished`.
   - Audio player becomes available with the full session audio.

### Copy / export

- **复制中文 / 复制英文** — joins all done segments' zh/en with `\n\n`, writes to clipboard, toast confirm.
- **含时间戳** — like the above but each line prefixed with `[MM:SS.c → MM:SS.c]`.
- **导出 SRT** — generates SRT subtitle file from segments (zh primary, en as second cue if present), downloads as `.srt`.
- **保存音频** — exports session audio as WAV.
- **Per-segment copy** — same as global, scoped to one segment. Button shows ✓ for 1.2s after click.
- **Per-segment WAV** — exports just that segment's audio slice.

### Playback

- Click time pill on a segment → seeks audio player to segment start, doesn't auto-play.
- Click play → playback plays through; the segment under the playhead becomes "active" (highlighted ring).
- Disabled during recording.

### Settings reload

Applying settings always triggers a model reload: status → `initializing` (~1.4s) → `idle`, toast "模型已就绪". Disable Start while initializing.

### Word correction

Rules apply post-ASR, pre-LLM. Stored as ordered list of `{from, to}` pairs. Empty rows ignored.

### Mode switch

Toggling simple ↔ detailed:
- In prototype: just swaps the React tree.
- In real Tauri/Electron app: also resize the OS window (~352×220 for simple, restore size for detailed) and toggle always-on-top.
- Persist last-used mode across launches.

### Keyboard

- `⌘R` / `Ctrl+R`: start/stop recording (must be intercepted before the browser refresh shortcut)
- `Esc`: close modal
- `Space`: play/pause when audio player focused
- `↑↓`: navigate segment list when focused

---

## State Management

Top-level state (a single store / context is fine):

| Key | Type | Notes |
|---|---|---|
| `status` | enum | `idle` `initializing` `recording` `processing` `error` `finished` |
| `recording` | boolean | derived but kept explicit for animation |
| `deviceId` | string | currently selected input device |
| `devices` | `[{id, label, kind}]` | system input devices, refreshed on focus |
| `modelReady` | boolean | gates the Start button |
| `settings` | `{vad, asr, llm}` | persisted to app data dir |
| `rules` | `[{from, to}]` | persisted |
| `autoCopy` | boolean | persisted |
| `showEnglish` | boolean | persisted |
| `segments` | `[{id, start, end, raw, zh, en, status, audioRange}]` | session-scoped, cleared on "清空结果" |
| `activeId` | string \| null | which segment the audio player is currently inside |
| `playPos`, `playing`, `totalDuration` | numbers/bool | audio player state |
| `uiMode` | `"simple" \| "detailed"` | persisted |
| `toasts` | `[{id, kind, msg}]` | ephemeral |

State transitions:

- Start recording: `idle` → `recording`; create empty session.
- VAD endpoint detected: append `{status: "processing"}` segment; ASR pipeline fills it in.
- Stop: `recording` → `processing` until pipeline drains → `finished`.
- Apply settings: any → `initializing` → `idle`.
- Error from pipeline: → `error`, toast.

---

## Design Tokens

All from `index.html` `:root`. Copy these verbatim into the host design system, or map onto its closest equivalents.

### Colors

```
--bg-canvas:    #f6f8f6   /* app background */
--bg-app:       #fbfcfb   /* control rail */
--bg-card:      #ffffff   /* cards, modals, dropdowns */
--bg-soft:      #f1f5f2   /* hover, disabled, time pills */
--bg-softer:    #f7f9f7

--line:         #e6ece7   /* default border */
--line-strong:  #d6ddd7   /* hover border */

--ink:          #1d2622   /* primary text */
--ink-2:        #4a5651   /* secondary text */
--ink-3:        #7a857f   /* tertiary, labels */
--ink-4:        #a4ada8   /* hints, captions */

--primary:        #1aa181   /* mint/teal — brand */
--primary-deep:   #138a6d   /* gradient end + hover */
--primary-soft:   #e6f6f0   /* chip/active backgrounds */
--primary-softer: #f1faf6   /* record card top gradient */

--accent:   #f6c46a   /* warm accent, sparingly */
--danger:   #e0584a
--danger-soft: #fbeae7
--warning:  #c98a2b
```

### Typography

- Sans: `Noto Sans SC` (CJK), fallback `HarmonyOS Sans SC`, `PingFang SC`, `Microsoft YaHei`, system
- Display (latin numerals/labels): `Geist`
- Mono (timers, time pills, kbd): `Geist Mono`
- Antialiasing: `-webkit-font-smoothing: antialiased`
- OpenType: `font-feature-settings: "ss01", "cv11"`

Scale used:

| Role | Size / weight / line |
|---|---|
| App title | 14.5px / 600 |
| Section labels (uppercase) | 11.5px / 500, `letter-spacing: 0.04em`, uppercase |
| Body | 14–15px / 400–500 / 1.6–1.7 |
| Segment zh | 15px / 400 / 1.7 |
| Segment en | 14px / 400 / 1.7 |
| Timer (record) | 30px mono / 500 |
| Timer (simple) | 24px mono / 500 |
| Time pill | 11px mono |
| Status chip | 12px / 500 |
| Caption / hint | 11px / 400 |

### Spacing

Uses a loose 4px-derived scale: 4, 6, 8, 10, 12, 14, 16, 18, 20, 22, 24, 26, 32. Most padding lives at 14–24.

### Radius

```
--radius-sm: 8px    /* small chips, kbd */
--radius:    12px   /* default buttons, inputs */
--radius-lg: 16px   /* cards (segment list) */
--radius-xl: 20px   /* modal, large surfaces */
```

Record card: `18px`. Simple-mode card: `18px`. Compact record card inside simple mode: `14px`.

### Shadows

```
--shadow-sm: 0 1px 2px rgba(20,40,32,0.04), 0 1px 1px rgba(20,40,32,0.03)
--shadow-md: 0 4px 14px rgba(20,40,32,0.05), 0 1px 2px rgba(20,40,32,0.04)
--shadow-lg: 0 18px 50px rgba(20,40,32,0.10), 0 4px 14px rgba(20,40,32,0.05)
```

Special: primary button has `box-shadow: 0 4px 14px rgba(20,161,129,0.22)`, hover deepens to `0.32`.

### Motion

- Default transition: `120–140ms ease` for hover; `220–240ms cubic-bezier(.4,0,.2,1)` for layout/structural.
- `pulseDot` 1.0–1.4s ease-in-out infinite.
- `fadeUp` 220ms — segment cards on enter.
- `shimmer` 1.6s linear infinite — processing-state placeholder (`background-position` -200% → 200%).
- `spin` 0.9s linear infinite — model-loading spinner.

---

## Assets

The prototype uses **no external image assets** — the logo is an inline SVG (sound-wave glyph), all icons are hand-rolled inline SVGs in `src/icons.jsx` (mic, stop, copy, check, download, save, clear, wand, settings, languages, clock, device, chevron, x, etc.).

Fonts are loaded from Google Fonts CDN: Noto Sans SC, Geist, Geist Mono. In a real desktop build, **bundle the fonts locally** — don't depend on the network.

---

## Files

Bundled in `design_files/`:

```
design_files/
├── index.html                 # entry, CSS variables, font loading
├── tweaks-panel.jsx           # design-time only, do NOT ship
└── src/
    ├── App.jsx                # top-level layout + state orchestration
    ├── ControlPanel.jsx       # left rail (detailed) + simple mode widget
    ├── SegmentList.jsx        # segment cards
    ├── AudioPlayer.jsx        # bottom audio scrubber
    ├── Waveform.jsx           # canvas waveform viz
    ├── SettingsModal.jsx      # VAD/ASR/LLM settings
    ├── RulesModal.jsx         # word correction rules
    ├── Toast.jsx              # toast stack
    ├── primitives.jsx         # Button, Switch, Dropdown, Modal, Icon wrappers
    ├── icons.jsx              # inline SVG icon set
    └── data.jsx               # mock segments, mock devices, time helpers
```

Open `index.html` directly in a browser to view the prototype. Tweaks panel (gear icon top-right of preview) lets you flip between simple/detailed mode and tune visual variables.

---

## Implementation Checklist

- [ ] Wire real microphone capture (target platform: WebRTC `getUserMedia` for Electron, `cpal`/native for Tauri).
- [ ] Integrate VAD (e.g. Silero VAD onnx) for endpoint detection.
- [ ] Integrate streaming ASR backend (Paraformer / Whisper / SenseVoice — settings expose this).
- [ ] Integrate LLM polish + EN translation (local or remote, configurable).
- [ ] Persist `settings`, `rules`, `autoCopy`, `showEnglish`, `uiMode` to platform-appropriate storage.
- [ ] Implement system clipboard write + native toasts.
- [ ] Implement WAV / SRT export (real session audio).
- [ ] Implement OS-window resize + always-on-top for simple mode.
- [ ] Implement global hotkey ⌘R (with permission UX on macOS).
- [ ] Bundle fonts locally; remove the Google Fonts `<link>`.
- [ ] Strip `tweaks-panel.jsx` and the `Tweaks` toggle.
- [ ] Replace prototype primitives with the host design-system equivalents (matching tokens above).
- [ ] Localize: prototype copy is zh-CN. Add an i18n layer if EN is needed in-product.
