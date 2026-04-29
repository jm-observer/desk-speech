#[path = "../src/audio_buffer.rs"]
mod audio_buffer;

use audio_buffer::{RollingAudioBuffer, MAX_AUDIO_SAMPLES, SAMPLE_RATE};

#[test]
fn keeps_600s_window_limit() {
    let mut buf = RollingAudioBuffer::new();
    buf.push_samples(&vec![0.1; MAX_AUDIO_SAMPLES]);
    assert_eq!(buf.len(), MAX_AUDIO_SAMPLES);
    assert_eq!(buf.global_start_sample(), 0);

    buf.push_samples(&vec![0.2; SAMPLE_RATE]);
    assert_eq!(buf.len(), MAX_AUDIO_SAMPLES);
    assert_eq!(buf.global_start_sample(), SAMPLE_RATE as u64);
}

#[test]
fn rolling_boundary_600_601_seconds() {
    let mut buf = RollingAudioBuffer::new();
    buf.push_samples(&vec![1.0; SAMPLE_RATE * 601]);

    assert_eq!(buf.len(), MAX_AUDIO_SAMPLES);
    assert_eq!(buf.global_start_sample(), SAMPLE_RATE as u64);
    assert_eq!(buf.global_end_sample(), (SAMPLE_RATE * 601) as u64);
}

#[test]
fn snapshot_range_rejects_out_of_window_request() {
    let mut buf = RollingAudioBuffer::new();
    buf.push_samples(&vec![1.0; SAMPLE_RATE * 601]);

    assert!(buf.snapshot_range(0, 100).is_none());

    let end = buf.global_end_sample();
    let start = end - 160;
    let ok = buf.snapshot_range(start, end);
    assert!(ok.is_some());
    assert_eq!(ok.unwrap().len(), 160);
}
