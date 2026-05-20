use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::extract::{DefaultBodyLimit, Multipart, State};
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::{get, post};
use axum::Router;
use clap::Parser;
use serde::Serialize;
use sherpa_onnx::{
    OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig, OfflineWhisperModelConfig,
};

#[derive(Parser, Debug, Clone)]
#[command(about = "Minimal OpenAI-compatible ASR HTTP service backed by sherpa-onnx")]
struct Args {
    #[arg(long, default_value = "0.0.0.0:8080")]
    bind: String,

    #[arg(long)]
    model_dir: PathBuf,

    /// sense-voice | whisper-turbo
    #[arg(long, default_value = "sense-voice")]
    model: String,

    /// Whisper 解码语言，仅 whisper-turbo 生效；留空为自动检测
    #[arg(long, default_value = "")]
    language: String,

    #[arg(long, default_value_t = 2)]
    num_threads: i32,

    #[arg(long, default_value_t = 50 * 1024 * 1024)]
    max_body_bytes: usize,
}

struct AppState {
    args: Args,
    recognizer: Mutex<OfflineRecognizer>,
}

fn build_config(args: &Args) -> anyhow::Result<OfflineRecognizerConfig> {
    let p = |sub: &str| -> Option<String> {
        let path = args.model_dir.join(sub);
        path.exists().then(|| path.to_string_lossy().into_owned())
    };
    let mut config = OfflineRecognizerConfig::default();
    match args.model.as_str() {
        "sense-voice" => {
            config.model_config.sense_voice = OfflineSenseVoiceModelConfig {
                model: p("model.int8.onnx"),
                language: Some("auto".into()),
                use_itn: true,
            };
            config.model_config.tokens = p("tokens.txt");
        }
        "whisper-turbo" => {
            let language = (!args.language.is_empty()).then(|| args.language.clone());
            config.model_config.whisper = OfflineWhisperModelConfig {
                encoder: p("turbo-encoder.onnx"),
                decoder: p("turbo-decoder.onnx"),
                language,
                task: Some("transcribe".into()),
                ..Default::default()
            };
            config.model_config.tokens = p("turbo-tokens.txt");
            config.model_config.model_type = Some("whisper".into());
        }
        other => anyhow::bail!("unsupported --model: {other} (expected 'sense-voice' or 'whisper-turbo')"),
    }
    config.model_config.num_threads = args.num_threads;
    Ok(config)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let args = Args::parse();
    tracing::info!(model = %args.model, dir = ?args.model_dir, "loading recognizer");
    let config = build_config(&args)?;
    let recognizer = OfflineRecognizer::create(&config)
        .ok_or_else(|| anyhow::anyhow!("failed to create recognizer; check --model-dir and model files"))?;

    let max_body = args.max_body_bytes;
    let state = Arc::new(AppState {
        args: args.clone(),
        recognizer: Mutex::new(recognizer),
    });

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/models", get(list_models))
        .route("/v1/audio/transcriptions", post(transcribe))
        .layer(DefaultBodyLimit::max(max_body))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&args.bind).await?;
    tracing::info!("listening on http://{}", args.bind);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn healthz() -> &'static str {
    "ok"
}

#[derive(Serialize)]
struct ModelEntry {
    id: String,
    object: &'static str,
}

#[derive(Serialize)]
struct ModelList {
    object: &'static str,
    data: Vec<ModelEntry>,
}

async fn list_models(State(state): State<Arc<AppState>>) -> Json<ModelList> {
    Json(ModelList {
        object: "list",
        data: vec![ModelEntry {
            id: state.args.model.clone(),
            object: "model",
        }],
    })
}

#[derive(Serialize)]
struct TranscriptionResponse {
    text: String,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: ErrorBody,
}

#[derive(Serialize)]
struct ErrorBody {
    message: String,
    r#type: &'static str,
}

type ApiError = (StatusCode, Json<ErrorResponse>);

fn err(status: StatusCode, kind: &'static str, msg: impl Into<String>) -> ApiError {
    (
        status,
        Json(ErrorResponse {
            error: ErrorBody { message: msg.into(), r#type: kind },
        }),
    )
}

async fn transcribe(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<TranscriptionResponse>, ApiError> {
    let mut audio_bytes: Option<Vec<u8>> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| err(StatusCode::BAD_REQUEST, "invalid_request", format!("multipart: {e}")))?
    {
        if field.name().unwrap_or("") == "file" {
            let bytes = field
                .bytes()
                .await
                .map_err(|e| err(StatusCode::BAD_REQUEST, "invalid_request", format!("read file: {e}")))?;
            audio_bytes = Some(bytes.to_vec());
        }
        // 其它字段（model/language/response_format/prompt）目前忽略，
        // 服务端通过启动参数固定模型；如需运行时切换请扩展此处。
    }

    let bytes = audio_bytes.ok_or_else(|| err(StatusCode::BAD_REQUEST, "invalid_request", "missing field 'file'"))?;
    let samples = decode_wav_16k_mono(&bytes)
        .map_err(|e| err(StatusCode::BAD_REQUEST, "invalid_request", e))?;

    let text = tokio::task::spawn_blocking({
        let state = state.clone();
        move || -> anyhow::Result<String> {
            let recognizer = state.recognizer.lock().expect("recognizer mutex poisoned");
            let stream = recognizer.create_stream();
            stream.accept_waveform(16000, &samples);
            recognizer.decode(&stream);
            let result = stream
                .get_result()
                .ok_or_else(|| anyhow::anyhow!("recognizer returned no result"))?;
            Ok(result.text.trim().to_string())
        }
    })
    .await
    .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, "server_error", format!("join: {e}")))?
    .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, "server_error", e.to_string()))?;

    Ok(Json(TranscriptionResponse { text }))
}

fn decode_wav_16k_mono(bytes: &[u8]) -> Result<Vec<f32>, String> {
    let cursor = std::io::Cursor::new(bytes);
    let reader = hound::WavReader::new(cursor).map_err(|e| format!("not a WAV file: {e}"))?;
    let spec = reader.spec();
    if spec.sample_rate != 16000 {
        return Err(format!("sample_rate must be 16000, got {}", spec.sample_rate));
    }
    if spec.channels != 1 {
        return Err(format!("channels must be 1 (mono), got {}", spec.channels));
    }
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let denom = (1u64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .into_samples::<i32>()
                .map(|s| s.map(|v| v as f32 / denom))
                .collect::<Result<_, _>>()
                .map_err(|e| format!("decode pcm: {e}"))?
        }
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .collect::<Result<_, _>>()
            .map_err(|e| format!("decode float: {e}"))?,
    };
    Ok(samples)
}
