//! Speaker (voice timbre) gating: enroll a target voiceprint, then only let
//! VAD segments whose embedding matches the target through to ASR.

use log::{info, warn};
use sherpa_onnx::{SpeakerEmbeddingExtractor, SpeakerEmbeddingExtractorConfig};

/// Default cosine-similarity threshold for accepting a segment as the target speaker.
pub(crate) const DEFAULT_SPEAKER_THRESHOLD: f32 = 0.5;

/// Runtime speaker-gating state. Held behind an `Arc<RwLock<_>>` in `AppState`.
pub(crate) struct SpeakerState {
    /// Embedding extractor (loaded once at init). `None` if the model is missing.
    pub(crate) extractor: Option<SpeakerEmbeddingExtractor>,
    /// Enrolled target voiceprint. `None` until the user enrolls.
    pub(crate) target: Option<Vec<f32>>,
    /// Whether gating is active (auto-enabled after enrollment).
    pub(crate) enabled: bool,
    /// Cosine-similarity acceptance threshold.
    pub(crate) threshold: f32,
}

impl Default for SpeakerState {
    fn default() -> Self {
        Self {
            extractor: None,
            target: None,
            enabled: false,
            threshold: DEFAULT_SPEAKER_THRESHOLD,
        }
    }
}

impl SpeakerState {
    /// True if a target is enrolled.
    pub(crate) fn is_enrolled(&self) -> bool {
        self.target.is_some()
    }
}

/// Build the speaker-embedding extractor from `speaker-embedding.onnx` in the
/// resource dir. Runs on CPU (model is tiny ~28MB; keeps GPU VRAM for Whisper).
pub(crate) fn build_speaker_extractor() -> Option<SpeakerEmbeddingExtractor> {
    let path = crate::resource_dir().join("speaker-embedding.onnx");
    if !path.exists() {
        warn!("[speaker] model not found: {path:?} — speaker gating disabled");
        return None;
    }
    let cfg = SpeakerEmbeddingExtractorConfig {
        model: Some(path.to_string_lossy().into_owned()),
        num_threads: 1,
        debug: false,
        provider: Some("cpu".to_string()),
    };
    match SpeakerEmbeddingExtractor::create(&cfg) {
        Some(ext) => {
            info!("[speaker] extractor created (dim={})", ext.dim());
            Some(ext)
        }
        None => {
            warn!("[speaker] failed to create extractor from {path:?}");
            None
        }
    }
}

/// Compute the embedding for a 16 kHz mono `samples` slice.
pub(crate) fn embed(extractor: &SpeakerEmbeddingExtractor, samples: &[f32]) -> Option<Vec<f32>> {
    let stream = extractor.create_stream()?;
    stream.accept_waveform(16000, samples);
    stream.input_finished();
    if !extractor.is_ready(&stream) {
        return None;
    }
    extractor.compute(&stream)
}

/// Cosine similarity between two equal-length embeddings.
pub(crate) fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return -1.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        return -1.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// Serialize an embedding to a compact CSV string for DB persistence.
pub(crate) fn embedding_to_csv(emb: &[f32]) -> String {
    emb.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",")
}

/// Parse a CSV embedding string back into a vector.
pub(crate) fn parse_embedding(csv: &str) -> Option<Vec<f32>> {
    let v: Vec<f32> = csv
        .split(',')
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.trim().parse::<f32>().ok())
        .collect();
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}
