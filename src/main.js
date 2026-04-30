const { invoke, convertFileSrc } = window.__TAURI__.core;
const { save, ask } = window.__TAURI__.dialog;
const { open: openUrl } = window.__TAURI__.shell;

const AUTO_COPY_STORAGE_KEY = "auto-copy-enabled";
const AUTO_COPY_MIN_INTERVAL_MS = 500;

const startBtn = document.querySelector("#start-btn");
const stopBtn = document.querySelector("#stop-btn");
const clearBtn = document.querySelector("#clear-btn");
const correctionBtn = document.querySelector("#correction-btn");
const statusEl = document.querySelector("#status");
const recordingIndicator = document.querySelector("#recording-indicator");
const elapsedTimer = document.querySelector("#elapsed-timer");
const resultsEl = document.querySelector("#results");
const resultsBody = document.querySelector("#results-body");
const copyTextBtn = document.querySelector("#copy-text-btn");
const copyTimedBtn = document.querySelector("#copy-timed-btn");
const exportSrtBtn = document.querySelector("#export-srt-btn");
const saveAllBtn = document.querySelector("#save-all-btn");
const playerWrapper = document.querySelector("#player-wrapper");
const player = document.querySelector("#player");
const settingsBtn = document.querySelector("#settings-btn");
const settingsModal = document.querySelector("#settings-modal");
const setThreshold = document.querySelector("#set-threshold");
const setMinSilence = document.querySelector("#set-min-silence");
const setMinSpeech = document.querySelector("#set-min-speech");
const setMaxSpeech = document.querySelector("#set-max-speech");
const setNumThreads = document.querySelector("#set-num-threads");
const settingsApplyBtn = document.querySelector("#settings-apply");
const settingsCancelBtn = document.querySelector("#settings-cancel");
const deviceSelect = document.querySelector("#device-select");
const autoCopyToggle = document.querySelector("#auto-copy-toggle");
const correctionModal = document.querySelector("#correction-modal");
const correctionBody = document.querySelector("#correction-body");
const correctionCloseBtn = document.querySelector("#correction-close-btn");
const correctionAddBtn = document.querySelector("#correction-add-btn");
const newSourceInput = document.querySelector("#new-source");
const newTargetInput = document.querySelector("#new-target");
const newPriorityInput = document.querySelector("#new-priority");
const newEnabledInput = document.querySelector("#new-enabled");

let recording = false;
let pollTimer = null;
let elapsedInterval = null;
let lastSegments = [];
let modelsReady = false;
let recordingStartTime = null;
let hasDevices = false;
let currentSessionId = null;
let tailAfterId = 0;
let correctionRules = [];
let lastAutoCopySign = "";
let lastAutoCopyAtMs = 0;
let autoStarted = false;

startBtn.disabled = true;
statusEl.textContent = "Loading models...";
statusEl.className = "status status-working";

autoCopyToggle.checked = localStorage.getItem(AUTO_COPY_STORAGE_KEY) !== "0";
autoCopyToggle.addEventListener("change", () => {
  localStorage.setItem(AUTO_COPY_STORAGE_KEY, autoCopyToggle.checked ? "1" : "0");
});

async function loadInitialData() {
  await Promise.all([loadCorrectionRules(), loadLatestSession()]);
}

async function loadLatestSession() {
  try {
    const sessions = await invoke("list_sessions", { page: 0, pageSize: 1 });
    if (!sessions.length) {
      currentSessionId = null;
      tailAfterId = 0;
      return;
    }
    currentSessionId = sessions[0].id;
    const segments = await invoke("list_session_segments", {
      sessionId: currentSessionId,
      page: 0,
      pageSize: 200,
    });
    applyDbSegments(segments, false);
  } catch (err) {
    flashStatus(`Load history error: ${err}`, true);
  }
}

async function loadCorrectionRules() {
  const rows = await invoke("list_correction_rules");
  correctionRules = rows;
  renderCorrectionRules();
}

function renderCorrectionRules() {
  correctionBody.innerHTML = "";
  correctionRules.forEach((rule) => {
    const tr = document.createElement("tr");
    tr.innerHTML = `
      <td><input type="checkbox" class="rule-enabled" data-id="${rule.id}" ${rule.enabled ? "checked" : ""} /></td>
      <td>${escapeHtml(rule.source)}</td>
      <td>${escapeHtml(rule.target)}</td>
      <td><input type="number" class="rule-priority" data-id="${rule.id}" value="${rule.priority}" step="1" /></td>
      <td><button class="rule-del-btn" data-id="${rule.id}">删除</button></td>
    `;
    correctionBody.appendChild(tr);
  });
}

correctionBtn.addEventListener("click", async () => {
  try {
    await loadCorrectionRules();
    correctionModal.style.display = "flex";
  } catch (err) {
    flashStatus(`Load rules error: ${err}`, true);
  }
});

correctionCloseBtn.addEventListener("click", () => {
  correctionModal.style.display = "none";
});

correctionModal.addEventListener("click", (e) => {
  if (e.target === correctionModal) {
    correctionModal.style.display = "none";
  }
});

correctionAddBtn.addEventListener("click", async () => {
  const source = newSourceInput.value.trim();
  const target = newTargetInput.value.trim();
  const priority = parseInt(newPriorityInput.value, 10);
  if (!source) {
    flashStatus("源词不能为空", true);
    return;
  }
  if (Number.isNaN(priority)) {
    flashStatus("优先级必须是整数", true);
    return;
  }

  try {
    await invoke("create_correction_rule", {
      source,
      target,
      priority,
      enabled: newEnabledInput.checked,
    });
    await invoke("reload_correction_rules");
    await loadCorrectionRules();
    newSourceInput.value = "";
    newTargetInput.value = "";
    newPriorityInput.value = "100";
    newEnabledInput.checked = true;
    flashStatus("规则已新增");
  } catch (err) {
    flashStatus(`Create rule error: ${err}`, true);
  }
});

correctionBody.addEventListener("change", async (e) => {
  const enabledEl = e.target.closest(".rule-enabled");
  const priorityEl = e.target.closest(".rule-priority");
  if (!enabledEl && !priorityEl) {
    return;
  }

  const id = parseInt((enabledEl || priorityEl).dataset.id, 10);
  const rule = correctionRules.find((r) => r.id === id);
  if (!rule) {
    return;
  }

  const priorityInput = correctionBody.querySelector(`.rule-priority[data-id="${id}"]`);
  const enabledInput = correctionBody.querySelector(`.rule-enabled[data-id="${id}"]`);
  const priority = parseInt(priorityInput.value, 10);
  if (Number.isNaN(priority)) {
    flashStatus("优先级必须是整数", true);
    priorityInput.value = String(rule.priority);
    return;
  }

  try {
    await invoke("update_correction_rule", {
      id,
      source: rule.source,
      target: rule.target,
      priority,
      enabled: enabledInput.checked,
    });
    await invoke("reload_correction_rules");
    await loadCorrectionRules();
  } catch (err) {
    flashStatus(`Update rule error: ${err}`, true);
  }
});

correctionBody.addEventListener("click", async (e) => {
  const btn = e.target.closest(".rule-del-btn");
  if (!btn) {
    return;
  }

  const id = parseInt(btn.dataset.id, 10);
  try {
    await invoke("delete_correction_rule", { id });
    await invoke("reload_correction_rules");
    await loadCorrectionRules();
    flashStatus("规则已删除");
  } catch (err) {
    flashStatus(`Delete rule error: ${err}`, true);
  }
});

async function loadDevices() {
  try {
    const devices = await invoke("list_input_devices");
    deviceSelect.innerHTML = "";
    if (devices.length === 0) {
      hasDevices = false;
      const opt = document.createElement("option");
      opt.value = "";
      opt.textContent = "No microphone found";
      deviceSelect.appendChild(opt);
      deviceSelect.disabled = true;
      startBtn.disabled = true;
      statusEl.textContent = "No microphone detected. Please connect a microphone and restart.";
      statusEl.className = "status status-error";
      return;
    }
    hasDevices = true;
    const selected = await invoke("get_selected_device");
    devices.forEach((d) => {
      const opt = document.createElement("option");
      opt.value = d.name;
      opt.textContent = d.is_default ? `${d.name} (default)` : d.name;
      if (selected ? d.name === selected : d.is_default) {
        opt.selected = true;
      }
      deviceSelect.appendChild(opt);
    });
    deviceSelect.disabled = false;
    if (modelsReady) {
      startBtn.disabled = false;
      correctionBtn.disabled = false;
      statusEl.textContent = "";
      statusEl.className = "status";
      maybeAutoStartRecording();
    }
  } catch (err) {
    deviceSelect.innerHTML = "<option>Error loading devices</option>";
    deviceSelect.disabled = true;
  }
}

deviceSelect.addEventListener("change", async () => {
  const name = deviceSelect.value || null;
  try {
    await invoke("set_input_device", { deviceName: name });
  } catch (err) {
    flashStatus(`Device error: ${err}`, true);
  }
});

loadDevices();
loadInitialData();

function pollInitStatus() {
  invoke("get_init_status")
    .then((res) => {
      if (res.status === 1) {
        modelsReady = true;
        startBtn.disabled = !hasDevices;
        settingsBtn.disabled = false;
        correctionBtn.disabled = false;
        clearBtn.disabled = false;
        if (hasDevices) {
          statusEl.textContent = "";
          statusEl.className = "status";
          maybeAutoStartRecording();
        }
      } else if (res.status === 2) {
        startBtn.disabled = true;
        settingsBtn.disabled = true;
        correctionBtn.disabled = true;
        statusEl.textContent = `Initialization failed: ${res.error}`;
        statusEl.className = "status status-error";
      } else {
        setTimeout(pollInitStatus, 300);
      }
    })
    .catch((err) => {
      startBtn.disabled = true;
      statusEl.textContent = `Init poll error: ${err}`;
      statusEl.className = "status status-error";
    });
}

pollInitStatus();

document.querySelectorAll("a[href]").forEach((a) => {
  a.addEventListener("click", (e) => {
    e.preventDefault();
    openUrl(e.currentTarget.href);
  });
});

copyTextBtn.addEventListener("click", async () => {
  const text = lastSegments.map((s) => s.text).join("\n");
  await invoke("copy_text_to_clipboard", { text });
  flashStatus("Text copied.");
});

copyTimedBtn.addEventListener("click", async () => {
  const lines = lastSegments.map((s) => `[${s.wall_start} --> ${s.wall_end}] ${s.text}`);
  await invoke("copy_text_to_clipboard", { text: lines.join("\n") });
  flashStatus("Text with time copied.");
});

exportSrtBtn.addEventListener("click", async () => {
  const filePath = await save({
    defaultPath: "subtitles.srt",
    filters: [{ name: "SubRip", extensions: ["srt"] }],
  });

  if (filePath === null) {
    return;
  }

  try {
    await invoke("export_srt", { path: filePath });
    flashStatus(`SRT saved to: ${filePath}`, false, 8000);
  } catch (err) {
    flashStatus(`Export error: ${err}`, true);
  }
});

saveAllBtn.addEventListener("click", async () => {
  const filePath = await save({
    defaultPath: "recording.wav",
    filters: [{ name: "WAV Audio", extensions: ["wav"] }],
  });

  if (filePath === null) {
    return;
  }

  try {
    await invoke("save_all_audio", { path: filePath });
    flashStatus(`Audio saved to: ${filePath}`, false, 8000);
  } catch (err) {
    flashStatus(`Save error: ${err}`, true);
  }
});

let lastActiveIdx = -1;

player.addEventListener("timeupdate", () => {
  const t = player.currentTime;
  const activeIdx = findSegmentIndex(t);

  if (activeIdx !== lastActiveIdx) {
    const rows = resultsBody.querySelectorAll("tr");
    if (lastActiveIdx >= 0 && lastActiveIdx < rows.length) {
      rows[lastActiveIdx].classList.remove("active");
    }
    if (activeIdx >= 0 && activeIdx < rows.length) {
      rows[activeIdx].classList.add("active");
      rows[activeIdx].scrollIntoView({ block: "nearest" });
    }
    lastActiveIdx = activeIdx;
  }
});

player.addEventListener("ended", () => {
  lastActiveIdx = -1;
  resultsBody.querySelectorAll("tr.active").forEach((tr) => tr.classList.remove("active"));
});

function findSegmentIndex(t) {
  let lo = 0;
  let hi = lastSegments.length - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const seg = lastSegments[mid];
    if (t < seg.start) {
      hi = mid - 1;
    } else if (t >= seg.end) {
      lo = mid + 1;
    } else {
      return mid;
    }
  }
  return -1;
}

resultsBody.addEventListener("click", (e) => {
  if (e.target.closest(".copy-seg-btn")) {
    e.stopPropagation();
    const btn = e.target.closest(".copy-seg-btn");
    const idx = parseInt(btn.dataset.idx, 10);
    if (idx >= 0 && idx < lastSegments.length) {
      copySegmentText(idx);
    }
    return;
  }

  if (e.target.closest(".save-seg-btn")) {
    e.stopPropagation();
    const btn = e.target.closest(".save-seg-btn");
    const idx = parseInt(btn.dataset.idx, 10);
    if (idx >= 0 && idx < lastSegments.length) {
      saveSegment(idx);
    }
    return;
  }

  const tr = e.target.closest("tr");
  if (!tr) {
    return;
  }
  const idx = parseInt(tr.dataset.idx, 10);
  if (idx >= 0 && idx < lastSegments.length) {
    player.pause();
    const t = Math.max(0, lastSegments[idx].start - 0.3);
    player.currentTime = t;
    player.addEventListener(
      "seeked",
      () => {
        player.play().catch(() => {});
      },
      { once: true }
    );
  }
});

async function copySegmentText(idx) {
  const seg = lastSegments[idx];
  try {
    await invoke("copy_text_to_clipboard", { text: seg.text });
    flashStatus("文本已复制");
  } catch (err) {
    flashStatus(`复制失败: ${err}`, true);
  }
}

async function saveSegment(idx) {
  const seg = lastSegments[idx];
  const wallPart = seg.wall_start.replace(/[:\s]/g, "-");
  const textPart = seg.text.replace(/[^\w一-鿿]/g, "_").slice(0, 30);
  const defaultName = `segment-${idx + 1}-${wallPart}-${textPart}.wav`;

  const filePath = await save({
    defaultPath: defaultName,
    filters: [{ name: "WAV Audio", extensions: ["wav"] }],
  });

  if (filePath === null) {
    return;
  }

  try {
    await invoke("save_segment_as_wav", {
      path: filePath,
      start: seg.start,
      end: seg.end,
    });
    flashStatus(`Saved: ${filePath}`, false, 8000);
  } catch (err) {
    flashStatus(`Save error: ${err}`, true);
  }
}

startBtn.addEventListener("click", async () => {
  if (recording || !modelsReady) {
    return;
  }

  try {
    await invoke("start_recording");
  } catch (err) {
    flashStatus(`Start error: ${err}`, true);
    return;
  }

  recording = true;
  currentSessionId = null;
  tailAfterId = 0;
  startBtn.style.display = "none";
  stopBtn.style.display = "";
  settingsBtn.disabled = true;
  correctionBtn.disabled = true;
  clearBtn.disabled = true;
  deviceSelect.disabled = true;
  recordingIndicator.style.display = "flex";
  playerWrapper.style.display = "none";
  player.src = "";
  lastActiveIdx = -1;
  statusEl.textContent = "";
  statusEl.className = "status";

  recordingStartTime = Date.now();
  elapsedInterval = setInterval(updateElapsedTimer, 1000);
  startPolling();
});

stopBtn.addEventListener("click", async () => {
  stopBtn.disabled = true;
  await invoke("stop_recording");
});

clearBtn.addEventListener("click", async () => {
  if (recording) {
    return;
  }

  const confirmed = await ask("This will clear all recognition results and recorded audio. Continue?", {
    title: "Clear All",
    kind: "warning",
  });
  if (!confirmed) {
    return;
  }

  try {
    await invoke("clear_results");
    lastSegments = [];
    currentSessionId = null;
    tailAfterId = 0;
    resultsBody.innerHTML = "";
    resultsEl.style.display = "none";
    playerWrapper.style.display = "none";
    player.src = "";
    lastActiveIdx = -1;
    statusEl.textContent = "";
    statusEl.className = "status";
    flashStatus("Cleared.");
  } catch (err) {
    flashStatus(`Clear error: ${err}`, true);
  }
});

function updateElapsedTimer() {
  if (!recordingStartTime) {
    return;
  }
  const secs = Math.floor((Date.now() - recordingStartTime) / 1000);
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  elapsedTimer.textContent = String(m).padStart(2, "0") + ":" + String(s).padStart(2, "0");
}

function startPolling() {
  if (pollTimer) {
    clearInterval(pollTimer);
  }
  pollTimer = setInterval(async () => {
    try {
      const state = await invoke("get_recording_state");
      applyLegacySegments(state.segments);
      await syncDbSegments();

      if (!state.recording && recording) {
        clearInterval(pollTimer);
        pollTimer = null;
        if (elapsedInterval) {
          clearInterval(elapsedInterval);
          elapsedInterval = null;
        }
        recording = false;
        startBtn.style.display = "";
        stopBtn.style.display = "none";
        stopBtn.disabled = false;
        settingsBtn.disabled = false;
        correctionBtn.disabled = false;
        clearBtn.disabled = false;
        deviceSelect.disabled = false;
        recordingIndicator.style.display = "none";

        const totalSecs = state.elapsed_secs;
        statusEl.textContent = `Done. ${lastSegments.length} segment(s) in ${totalSecs.toFixed(1)}s.`;
        statusEl.className = "status status-done";

        try {
          const audioPath = await invoke("get_recorded_audio_path");
          player.src = convertFileSrc(audioPath);
          playerWrapper.style.display = "block";
        } catch (err) {
          if (lastSegments.length > 0) {
            flashStatus(`Could not load playback: ${err}`, true);
          }
        }
      }
    } catch (err) {
      clearInterval(pollTimer);
      pollTimer = null;
      if (elapsedInterval) {
        clearInterval(elapsedInterval);
        elapsedInterval = null;
      }
      recording = false;
      startBtn.style.display = "";
      stopBtn.style.display = "none";
      stopBtn.disabled = false;
      settingsBtn.disabled = false;
      correctionBtn.disabled = false;
      clearBtn.disabled = false;
      deviceSelect.disabled = false;
      recordingIndicator.style.display = "none";
      statusEl.textContent = `Poll error: ${err}`;
      statusEl.className = "status status-error";
    }
  }, 200);
}

function applyLegacySegments(segments) {
  if (!segments.length) {
    return;
  }

  const normalized = segments.map((seg) => ({
    segment_id: seg.segment_id ?? null,
    update_type: seg.update_type ?? "append",
    id: null,
    start: seg.start,
    end: seg.end,
    wall_start: seg.wall_start,
    wall_end: seg.wall_end,
    text: seg.text,
    sign: `${seg.start}-${seg.end}-${seg.text}`,
  }));

  for (const seg of normalized) {
    if (seg.update_type === "replace" && seg.segment_id !== null) {
      const idx = lastSegments.findIndex((oldSeg) => oldSeg.segment_id === seg.segment_id);
      if (idx >= 0) {
        lastSegments[idx] = { ...lastSegments[idx], ...seg };
        renderSegments();
        maybeAutoCopy(seg);
        continue;
      }
    }

    if (lastSegments.some((oldSeg) => oldSeg.sign === seg.sign)) {
      continue;
    }
    appendSegment(seg);
    maybeAutoCopy(seg);
  }
}

async function syncDbSegments() {
  if (!currentSessionId) {
    const sessions = await invoke("list_sessions", { page: 0, pageSize: 1 });
    if (!sessions.length) {
      return;
    }
    currentSessionId = sessions[0].id;
    const existing = await invoke("list_session_segments", {
      sessionId: currentSessionId,
      page: 0,
      pageSize: 200,
    });
    applyDbSegments(existing, true);
    return;
  }

  const delta = await invoke("tail_session_segments", {
    sessionId: currentSessionId,
    afterId: tailAfterId,
    limit: 200,
  });
  applyDbSegments(delta, true);
}

function applyDbSegments(dbSegments, fromTail) {
  for (const seg of dbSegments) {
    const sign = `${seg.start_sec}-${seg.end_sec}-${seg.text_corrected}`;
    const normalized = {
      id: seg.id,
      start: seg.start_sec,
      end: seg.end_sec,
      wall_start: seg.wall_start,
      wall_end: seg.wall_end,
      text: seg.text_corrected,
      sign,
    };
    if (lastSegments.some((s) => (normalized.id !== null && s.id === normalized.id) || s.sign === sign)) {
      tailAfterId = Math.max(tailAfterId, seg.id);
      continue;
    }
    appendSegment(normalized);
    tailAfterId = Math.max(tailAfterId, seg.id);
    if (fromTail) {
      maybeAutoCopy(normalized);
    }
  }
}

function appendSegment(seg) {
  lastSegments.push(seg);
  renderSegments();
  resultsEl.style.display = "block";
  const lastRow = resultsBody.lastElementChild;
  if (lastRow) {
    lastRow.scrollIntoView({ behavior: "smooth", block: "end" });
  }
}

function renderSegments() {
  resultsBody.innerHTML = "";
  lastSegments.forEach((seg, idx) => {
    const tr = document.createElement("tr");
    tr.dataset.idx = idx;
    tr.innerHTML = `
      <td>${escapeHtml(stripYear(seg.wall_start))}</td>
      <td>${escapeHtml(stripYear(seg.wall_end))}</td>
      <td>${(seg.end - seg.start).toFixed(2)}s</td>
      <td>${escapeHtml(seg.text)}</td>
      <td class="seg-actions-cell">
        <button class="copy-seg-btn" data-idx="${idx}" title="Copy text">复制</button>
        <button class="save-seg-btn" data-idx="${idx}" title="Save as WAV">&#128190;</button>
      </td>
    `;
    resultsBody.appendChild(tr);
  });
}

async function maybeAutoCopy(seg) {
  if (!autoCopyToggle.checked) {
    return;
  }
  if (seg.sign === lastAutoCopySign) {
    return;
  }

  const now = Date.now();
  if (now - lastAutoCopyAtMs < AUTO_COPY_MIN_INTERVAL_MS) {
    return;
  }

  try {
    await invoke("copy_text_to_clipboard", { text: seg.text });
    lastAutoCopySign = seg.sign;
    lastAutoCopyAtMs = now;
    const preview = seg.text.length > 24 ? `${seg.text.slice(0, 24)}...` : seg.text;
    flashStatus(`已自动复制：${preview}`);
  } catch (err) {
    flashStatus(`自动复制失败: ${err}`, true);
  }
}

settingsBtn.addEventListener("click", async () => {
  if (!modelsReady || recording) {
    return;
  }

  try {
    const s = await invoke("get_settings");
    setThreshold.value = s.threshold;
    setMinSilence.value = s.min_silence_duration;
    setMinSpeech.value = s.min_speech_duration;
    setMaxSpeech.value = s.max_speech_duration;
    setNumThreads.value = s.num_threads;
    settingsModal.style.display = "flex";
  } catch (err) {
    flashStatus(`Settings error: ${err}`, true);
  }
});

settingsCancelBtn.addEventListener("click", () => {
  settingsModal.style.display = "none";
});

settingsModal.addEventListener("click", (e) => {
  if (e.target === settingsModal) {
    settingsModal.style.display = "none";
  }
});

settingsApplyBtn.addEventListener("click", async () => {
  const newSettings = {
    threshold: parseFloat(setThreshold.value),
    min_silence_duration: parseFloat(setMinSilence.value),
    min_speech_duration: parseFloat(setMinSpeech.value),
    max_speech_duration: parseFloat(setMaxSpeech.value),
    num_threads: parseInt(setNumThreads.value, 10),
  };

  if (Number.isNaN(newSettings.threshold) || newSettings.threshold < 0 || newSettings.threshold > 1) {
    flashStatus("Threshold must be between 0.0 and 1.0", true);
    return;
  }
  if (Number.isNaN(newSettings.min_silence_duration) || newSettings.min_silence_duration < 0) {
    flashStatus("Min silence duration must be >= 0", true);
    return;
  }
  if (Number.isNaN(newSettings.min_speech_duration) || newSettings.min_speech_duration < 0) {
    flashStatus("Min speech duration must be >= 0", true);
    return;
  }
  if (Number.isNaN(newSettings.max_speech_duration) || newSettings.max_speech_duration <= 0) {
    flashStatus("Max speech duration must be > 0", true);
    return;
  }
  if (Number.isNaN(newSettings.num_threads) || newSettings.num_threads < 1 || newSettings.num_threads > 16) {
    flashStatus("Threads must be between 1 and 16", true);
    return;
  }

  settingsApplyBtn.disabled = true;
  try {
    await invoke("apply_settings", { newSettings });
    settingsModal.style.display = "none";

    modelsReady = false;
    startBtn.disabled = true;
    settingsBtn.disabled = true;
    correctionBtn.disabled = true;
    statusEl.textContent = "Reloading models...";
    statusEl.className = "status status-working";
    pollInitStatus();
  } catch (err) {
    flashStatus(`Apply error: ${err}`, true);
  } finally {
    settingsApplyBtn.disabled = false;
  }
});

function maybeAutoStartRecording() {
  if (autoStarted || recording || !modelsReady || !hasDevices) {
    return;
  }
  autoStarted = true;
  startBtn.click();
}

function escapeHtml(text) {
  const el = document.createElement("span");
  el.textContent = text;
  return el.innerHTML;
}

function stripYear(wall) {
  const parts = wall.split(" ");
  return parts.length >= 2 ? parts[parts.length - 1] : wall;
}

let flashTimer = null;
function flashStatus(msg, isError, durationMs) {
  const prev = { text: statusEl.textContent, cls: statusEl.className };
  statusEl.textContent = msg;
  statusEl.className = isError ? "status status-error" : "status status-done";
  if (flashTimer) {
    clearTimeout(flashTimer);
  }
  flashTimer = setTimeout(() => {
    statusEl.textContent = prev.text;
    statusEl.className = prev.cls;
    flashTimer = null;
  }, durationMs || 2000);
}
