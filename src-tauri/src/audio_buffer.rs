use std::collections::VecDeque;

pub(crate) const MAX_AUDIO_WINDOW_SECS: usize = 120;
pub(crate) const SAMPLE_RATE: usize = 16000;
pub(crate) const MAX_AUDIO_SAMPLES: usize = MAX_AUDIO_WINDOW_SECS * SAMPLE_RATE;

#[derive(Clone)]
pub(crate) struct RollingAudioBuffer {
    samples: VecDeque<f32>,
    global_start_sample: u64,
}

impl RollingAudioBuffer {
    pub(crate) fn new() -> Self {
        Self {
            samples: VecDeque::with_capacity(MAX_AUDIO_SAMPLES),
            global_start_sample: 0,
        }
    }

    pub(crate) fn clear(&mut self) {
        self.samples.clear();
        self.global_start_sample = 0;
    }

    pub(crate) fn push_samples(&mut self, input: &[f32]) {
        if input.is_empty() {
            return;
        }

        self.samples.extend(input.iter().copied());
        while self.samples.len() > MAX_AUDIO_SAMPLES {
            self.samples.pop_front();
            self.global_start_sample += 1;
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.samples.len()
    }

    pub(crate) fn global_start_sample(&self) -> u64 {
        self.global_start_sample
    }

    pub(crate) fn global_end_sample(&self) -> u64 {
        self.global_start_sample + self.samples.len() as u64
    }

    pub(crate) fn snapshot_all(&self) -> Vec<f32> {
        self.samples.iter().copied().collect()
    }

    pub(crate) fn snapshot_range(&self, global_start: u64, global_end: u64) -> Option<Vec<f32>> {
        if global_start >= global_end {
            return None;
        }

        let window_start = self.global_start_sample;
        let window_end = self.global_end_sample();
        if global_start < window_start || global_end > window_end {
            return None;
        }

        let start = (global_start - window_start) as usize;
        let end = (global_end - window_start) as usize;
        Some(self.samples.range(start..end).copied().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_fixed_window() {
        let mut buf = RollingAudioBuffer::new();
        let input = vec![1.0_f32; MAX_AUDIO_SAMPLES + 32];
        buf.push_samples(&input);
        assert_eq!(buf.len(), MAX_AUDIO_SAMPLES);
        assert_eq!(buf.global_start_sample(), 32);
    }

    #[test]
    fn range_outside_window_is_none() {
        let mut buf = RollingAudioBuffer::new();
        buf.push_samples(&vec![1.0; 100]);
        assert!(buf.snapshot_range(0, 101).is_none());
    }
}
