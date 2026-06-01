//! System-tray "bounce" + short audio notification.
//!
//! When a segment finishes both LLM polish (优化中文) and translation (英文翻译),
//! we briefly toggle the system-tray icon visibility twice to grab attention,
//! optionally accompanied by two short system "beeps" (滴滴).
//!
//! Why this and not `FlashWindowEx`:
//! - `FlashWindowEx` operates on the taskbar button. The frontend hides the
//!   window on minimize (`window.hide()` in `src/App.tsx`), which removes the
//!   taskbar button — so a Win32 flash would have nothing to flash. Tauri's
//!   built-in `request_user_attention` has the same limitation.
//! - The visible UI affordance when the window is minimized-to-tray is the
//!   tray icon at the bottom-right. Native tray icons don't have a flash API,
//!   so we simulate "bouncing" by alternating `set_icon(None)` (icon disappears)
//!   and `set_icon(Some(original))` (icon reappears). Two cycles ≈ "跳动两次".
//!
//! Beep: `MessageBeep(MB_ICONASTERISK)` plays the user's configured "Star"
//! event sound. It respects system volume + Focus Assist. The beep is also
//! gated by the desktop "提示音" switch (`LlmSettings::notify_sound`) so users
//! with desktop speakers + mic — where the chirp would be picked up by the
//! next recording segment — can turn it off from the control panel.

use std::time::Duration;

use log::{info, warn};

/// One full cycle = off → on. Two cycles ≈ "bounces twice".
const BOUNCE_CYCLES: usize = 2;
/// How long the icon stays hidden / visible in each half-cycle. Slower than
/// the first iteration (180ms) so the animation reads as "two distinct hops"
/// rather than a quick flicker.
const BOUNCE_STEP: Duration = Duration::from_millis(320);

pub fn bounce_tray_twice(app: &tauri::AppHandle, play_beep: bool) {
    info!(
        "[notify] bounce_tray_twice invoked (beep={} cycles={} step={:?})",
        play_beep, BOUNCE_CYCLES, BOUNCE_STEP
    );
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(tray) = app.tray_by_id("main") else {
            warn!("[notify] tray 'main' not found — was it built with with_id(\"main\")?");
            return;
        };
        let Some(icon) = app.default_window_icon().cloned().map(|i| i.to_owned()) else {
            warn!("[notify] default window icon missing, cannot restore after bounce");
            return;
        };
        for i in 0..BOUNCE_CYCLES {
            if let Err(e) = tray.set_icon(None) {
                warn!("[notify] set_icon(None) failed at cycle {i}: {e}");
                break;
            }
            if play_beep {
                play_beep_async();
            }
            tokio::time::sleep(BOUNCE_STEP).await;
            if let Err(e) = tray.set_icon(Some(icon.clone())) {
                warn!("[notify] set_icon(Some) failed at cycle {i}: {e}");
                break;
            }
            tokio::time::sleep(BOUNCE_STEP).await;
        }
        info!("[notify] tray bounce complete");
    });
}

/// Fire-and-forget short system event sound. Runs on a blocking pool thread
/// so the tray-animation task doesn't stall while the audio device plays.
#[cfg(windows)]
fn play_beep_async() {
    tauri::async_runtime::spawn_blocking(|| {
        use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
        use windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONASTERISK;
        // MessageBeep blocks until the sound starts (typically <50ms) and the
        // sound itself is short (100–300ms depending on the user's sound scheme).
        unsafe {
            MessageBeep(MB_ICONASTERISK);
        }
    });
}

#[cfg(not(windows))]
fn play_beep_async() {
    // No portable equivalent worth wiring up here; this app only ships on Windows.
}
