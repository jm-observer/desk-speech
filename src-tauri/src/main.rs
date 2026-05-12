#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use custom_utils::logger;
use log::LevelFilter::Info;

fn configure_linux_webkit_env() {
    #[cfg(target_os = "linux")]
    {
        let is_x11 = std::env::var("XDG_SESSION_TYPE")
            .map(|value| value.eq_ignore_ascii_case("x11"))
            .unwrap_or(false);
        if is_x11 && std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
    }
}

fn main() {
    configure_linux_webkit_env();
    let _logger_handle = logger::logger_feature("streaming-speech", "debug", Info, false).build();
    non_streaming_speech_recognition_from_microphone_lib::run();
}
