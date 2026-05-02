#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use custom_utils::logger;
use log::LevelFilter::Info;

fn main() {
    let _logger_handle = logger::logger_feature("streaming-speech", "debug", Info, false).build();
    non_streaming_speech_recognition_from_microphone_lib::run();
}
