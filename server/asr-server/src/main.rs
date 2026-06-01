use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, Multipart, State};
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::{get, post};
use axum::Router;
use clap::Parser;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sherpa_onnx::{
    OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig, OfflineWhisperModelConfig,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

mod vad;

#[derive(Parser, Debug, Clone)]
#[command(about = "Minimal OpenAI-compatible ASR HTTP service backed by sherpa-onnx")]
struct Args {
    /// 默认绑定 127.0.0.1（Plan C 安全默认值）。容器内仍需 0.0.0.0 才能被
    /// docker 端口转发到达——见 Dockerfile CMD / compose ports 注释。
    #[arg(long, default_value = "127.0.0.1:8091")]
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

    /// silero_vad.onnx 路径（Plan B）。镜像内由 Dockerfile COPY 到该位置。
    #[arg(long, default_value = "/opt/asr-server/silero_vad.onnx")]
    vad_model: PathBuf,

    /// ffmpeg 解码超时（秒，Plan A）
    #[arg(long, default_value_t = 60)]
    decode_timeout: u64,

    /// from-source 端点白名单前缀（逗号分隔，Plan C）。为空则 from-source 禁用。
    #[arg(long, value_delimiter = ',')]
    source_allowlist: Vec<PathBuf>,

    /// from-source HTTP 下载体积上限（字节，Plan C）
    #[arg(long, default_value_t = 100 * 1024 * 1024)]
    max_source_bytes: u64,

    /// from-source HTTP 下载整体超时（秒，Plan C）
    #[arg(long, default_value_t = 30)]
    source_fetch_timeout: u64,
}

struct AppState {
    args: Args,
    is_whisper: bool,
    recognizer: Mutex<OfflineRecognizer>,
    /// canonical 化后的白名单前缀（Plan C）。空 = from-source 禁用。
    allowlist: Vec<PathBuf>,
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
    let is_whisper = args.model == "whisper-turbo";
    tracing::info!(model = %args.model, dir = ?args.model_dir, "loading recognizer");
    let config = build_config(&args)?;
    let recognizer = OfflineRecognizer::create(&config)
        .ok_or_else(|| anyhow::anyhow!("failed to create recognizer; check --model-dir and model files"))?;

    // 白名单 canonical 化（Plan C）：避免 symlink / `..` 逃逸。无法 canonical 化的
    // 条目（如目录不存在）丢弃并 warn。
    let allowlist: Vec<PathBuf> = args
        .source_allowlist
        .iter()
        .filter_map(|p| match p.canonicalize() {
            Ok(c) => Some(c),
            Err(e) => {
                tracing::warn!("--source-allowlist entry {:?} dropped (canonicalize failed: {e})", p);
                None
            }
        })
        .collect();
    if allowlist.is_empty() {
        tracing::warn!("--source-allowlist empty: /v1/audio/transcriptions/from-source is DISABLED");
    } else {
        tracing::info!(?allowlist, "from-source enabled with allowlist");
    }
    if !args.vad_model.exists() {
        tracing::warn!("vad model not found at {:?}; vad=true requests will fail", args.vad_model);
    }

    let max_body = args.max_body_bytes;
    let bind = args.bind.clone();
    let state = Arc::new(AppState {
        args,
        is_whisper,
        recognizer: Mutex::new(recognizer),
        allowlist,
    });

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/models", get(list_models))
        .route("/v1/audio/transcriptions", post(transcribe))
        .route("/v1/audio/transcriptions/from-source", post(from_source))
        .layer(DefaultBodyLimit::max(max_body))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!("listening on http://{}", bind);
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
    /// 仅 vad=true 时存在（Plan B）；不传 vad 时省略，保持向后兼容。
    #[serde(skip_serializing_if = "Option::is_none")]
    segments: Option<Vec<SegmentOut>>,
}

#[derive(Serialize)]
struct SegmentOut {
    start: f64,
    end: f64,
    text: String,
}

#[derive(Serialize, Debug)]
struct ErrorResponse {
    error: ErrorBody,
}

#[derive(Serialize, Debug)]
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

fn parse_bool_field(s: &str) -> bool {
    matches!(s.trim().to_ascii_lowercase().as_str(), "true" | "1")
}

// ===================== /v1/audio/transcriptions (multipart) =====================

async fn transcribe(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<TranscriptionResponse>, ApiError> {
    let mut audio_bytes: Option<Vec<u8>> = None;
    let mut vad_flag = false;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| err(StatusCode::BAD_REQUEST, "invalid_request", format!("multipart: {e}")))?
    {
        match field.name().unwrap_or("") {
            "file" => {
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| err(StatusCode::BAD_REQUEST, "invalid_request", format!("read file: {e}")))?;
                audio_bytes = Some(bytes.to_vec());
            }
            "vad" => {
                let v = field
                    .text()
                    .await
                    .map_err(|e| err(StatusCode::BAD_REQUEST, "invalid_request", format!("read vad: {e}")))?;
                vad_flag = parse_bool_field(&v);
            }
            // 其它字段（model/language/response_format/prompt）忽略，
            // 服务端通过启动参数固定模型。
            _ => {}
        }
    }

    let bytes = audio_bytes.ok_or_else(|| err(StatusCode::BAD_REQUEST, "invalid_request", "missing field 'file'"))?;
    let samples = decode_any(&bytes, &state.args).await?;
    let resp = run_transcription(state, samples, vad_flag).await?;
    Ok(Json(resp))
}

// ===================== /v1/audio/transcriptions/from-source (JSON) =====================

#[derive(Deserialize)]
struct FromSourceRequest {
    source: String,
    #[serde(default)]
    vad: bool,
}

async fn from_source(
    State(state): State<Arc<AppState>>,
    Json(req): Json<FromSourceRequest>,
) -> Result<Json<TranscriptionResponse>, ApiError> {
    if state.allowlist.is_empty() {
        return Err(err(
            StatusCode::SERVICE_UNAVAILABLE,
            "endpoint_disabled",
            "from-source disabled; configure --source-allowlist",
        ));
    }

    // _tmp 持有 HTTP 下载临时文件的 Drop guard：函数返回（成功或出错）即删文件。
    let _tmp;
    let bytes: Vec<u8>;

    if let Some(rest) = req.source.strip_prefix("file://") {
        let path = validate_file_path(&req.source, rest)?;
        if !path.exists() {
            return Err(err(StatusCode::NOT_FOUND, "not_found", "source file not found"));
        }
        let canon = path
            .canonicalize()
            .map_err(|_| err(StatusCode::NOT_FOUND, "not_found", "source file not found"))?;
        if !path_in_allowlist(&canon, &state.allowlist) {
            return Err(err(StatusCode::FORBIDDEN, "forbidden_source", "path not in --source-allowlist"));
        }
        bytes = tokio::fs::read(&canon)
            .await
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, "server_error", format!("read source: {e}")))?;
    } else if req.source.starts_with("http://") || req.source.starts_with("https://") {
        let (b, guard) = fetch_http(
            &req.source,
            state.args.max_source_bytes,
            state.args.source_fetch_timeout,
        )
        .await?;
        bytes = b;
        _tmp = guard;
    } else {
        return Err(err(StatusCode::BAD_REQUEST, "invalid_request", "unsupported source scheme"));
    }

    let samples = decode_any(&bytes, &state.args).await?;
    let resp = run_transcription(state, samples, req.vad).await?;
    Ok(Json(resp))
}

/// `file://` 路径合法性校验（纯函数，不碰文件系统）。`rest` 是去掉 `file://`
/// 前缀后的部分，对 `file:///abs/path` 即 `/abs/path`。
fn validate_file_path(source: &str, rest: &str) -> Result<PathBuf, ApiError> {
    // 拒绝 URL 编码字符：避免 %2e%2e 之类绕过 canonical、以及多重解码歧义。
    if source.contains('%') {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "encoded file:// path not supported",
        ));
    }
    // file:///abs → rest 以 '/' 开头；否则形如 file://host/... 或缺斜杠，不支持。
    if !rest.starts_with('/') {
        return Err(err(StatusCode::BAD_REQUEST, "invalid_request", "unsupported file:// path"));
    }
    // 拒绝 Windows 风格 file:///C:/...（本服务只跑 GB10 linux）。
    if is_windows_style(rest) {
        return Err(err(StatusCode::BAD_REQUEST, "invalid_request", "unsupported file:// path"));
    }
    Ok(PathBuf::from(rest))
}

/// `/C:/...` 形态判定（去掉 file:// 后）。
fn is_windows_style(rest: &str) -> bool {
    let b = rest.as_bytes();
    b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':'
}

fn path_in_allowlist(canon: &Path, allow: &[PathBuf]) -> bool {
    allow.iter().any(|prefix| canon.starts_with(prefix))
}

/// 处理完即删的临时文件 guard。
struct TempFile(PathBuf);
impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// 流式下载 HTTP(S) 到临时文件，边读边累计字节数，超 `max_bytes` 立刻中止；
/// 整次受 `timeout_secs` 约束。返回文件内容 + Drop guard（调用方持有以延后删除）。
async fn fetch_http(url: &str, max_bytes: u64, timeout_secs: u64) -> Result<(Vec<u8>, TempFile), ApiError> {
    let dir = PathBuf::from("/tmp/asr-input");
    tokio::fs::create_dir_all(&dir).await.ok();
    let tmp_path = dir.join(format!("{}.bin", uuid::Uuid::new_v4()));
    let guard = TempFile(tmp_path.clone());

    let fetch = async {
        let resp = reqwest::get(url)
            .await
            .map_err(|e| err(StatusCode::BAD_REQUEST, "invalid_request", format!("fetch failed: {e}")))?;
        let mut file = tokio::fs::File::create(&tmp_path)
            .await
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, "server_error", format!("temp create: {e}")))?;
        let mut stream = resp.bytes_stream();
        let mut total: u64 = 0;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk
                .map_err(|e| err(StatusCode::BAD_REQUEST, "invalid_request", format!("fetch read: {e}")))?;
            total += chunk.len() as u64;
            if total > max_bytes {
                return Err(err(StatusCode::BAD_REQUEST, "invalid_request", "fetch too large"));
            }
            file.write_all(&chunk)
                .await
                .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, "server_error", format!("temp write: {e}")))?;
        }
        file.flush().await.ok();
        Ok::<(), ApiError>(())
    };

    match tokio::time::timeout(Duration::from_secs(timeout_secs), fetch).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e), // guard 在此 drop → 临时文件删除
        Err(_) => return Err(err(StatusCode::BAD_REQUEST, "invalid_request", "fetch timeout")),
    }

    let bytes = tokio::fs::read(&tmp_path)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, "server_error", format!("read temp: {e}")))?;
    Ok((bytes, guard))
}

// ===================== 解码：任意输入 → 16k mono f32 PCM (Plan A) =====================

async fn decode_any(bytes: &[u8], args: &Args) -> Result<Vec<f32>, ApiError> {
    let samples = if is_fast_wav(bytes) {
        decode_wav_16k_mono(bytes).map_err(|e| err(StatusCode::BAD_REQUEST, "invalid_request", e))?
    } else {
        ffmpeg_decode(bytes, args.decode_timeout).await?
    };
    // 解码出的音频 < 0.1 s（1600 样本 @16k）视为无效输入。
    if samples.len() < 1600 {
        return Err(err(StatusCode::BAD_REQUEST, "invalid_request", "decoded audio too short"));
    }
    Ok(samples)
}

/// 只读 WAV 头判定能否走快路径：RIFF/WAVE + fmt chunk 满足
/// channels==1 && sample_rate==16000 && bits∈{16,32} && audio_format∈{1=PCM,3=Float}。
/// 任何不满足 / 解析失败 → false（交给 ffmpeg 慢路径）。
fn is_fast_wav(b: &[u8]) -> bool {
    if b.len() < 44 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return false;
    }
    let mut off = 12usize;
    while off + 8 <= b.len() {
        let cid = &b[off..off + 4];
        let csz = u32::from_le_bytes([b[off + 4], b[off + 5], b[off + 6], b[off + 7]]) as usize;
        let body = off + 8;
        if cid == b"fmt " {
            if body + 16 > b.len() {
                return false;
            }
            let audio_format = u16::from_le_bytes([b[body], b[body + 1]]);
            let channels = u16::from_le_bytes([b[body + 2], b[body + 3]]);
            let sample_rate = u32::from_le_bytes([b[body + 4], b[body + 5], b[body + 6], b[body + 7]]);
            let bits = u16::from_le_bytes([b[body + 14], b[body + 15]]);
            return channels == 1
                && sample_rate == 16000
                && (bits == 16 || bits == 32)
                && (audio_format == 1 || audio_format == 3);
        }
        // chunk 按偶数字节对齐。
        off = body + csz + (csz & 1);
    }
    false
}

/// 走 ffmpeg：stdin 喂原始 bytes，stdout 拿 16k mono s16le PCM，全程不落盘。
async fn ffmpeg_decode(bytes: &[u8], timeout_secs: u64) -> Result<Vec<f32>, ApiError> {
    use tokio::process::Command;

    let mut child = Command::new("ffmpeg")
        .args([
            "-hide_banner", "-loglevel", "error",
            "-i", "pipe:0",
            "-f", "s16le", "-ar", "16000", "-ac", "1", "-acodec", "pcm_s16le",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, "server_error", format!("spawn ffmpeg: {e}")))?;

    let mut stdin = child.stdin.take().expect("piped stdin");
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");

    // 同时写 stdin / 读 stdout / 读 stderr，避免任一管道写满造成死锁。
    let input = bytes.to_vec();
    let write_task = tokio::spawn(async move {
        let _ = stdin.write_all(&input).await;
        // stdin 在此 drop → 向 ffmpeg 发 EOF
    });
    let out_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf).await;
        buf
    });
    let err_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf).await;
        buf
    });

    let status = match tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait()).await {
        Ok(s) => s.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, "server_error", format!("ffmpeg wait: {e}")))?,
        Err(_) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            return Err(err(StatusCode::BAD_REQUEST, "invalid_request", "decode timeout"));
        }
    };

    let _ = write_task.await;
    let stdout_buf = out_task.await.unwrap_or_default();
    let stderr_buf = err_task.await.unwrap_or_default();

    if !status.success() {
        let stderr = String::from_utf8_lossy(&stderr_buf);
        let snippet: String = stderr.chars().take(200).collect();
        return Err(err(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            format!("ffmpeg decode failed: {snippet}"),
        ));
    }

    let samples: Vec<f32> = stdout_buf
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
        .collect();
    Ok(samples)
}

// ===================== 识别（含 VAD 切段，Plan B） =====================

async fn run_transcription(
    state: Arc<AppState>,
    samples: Vec<f32>,
    vad_flag: bool,
) -> Result<TranscriptionResponse, ApiError> {
    if vad_flag && !state.args.vad_model.exists() {
        return Err(err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            format!("vad model not available: {:?}", state.args.vad_model),
        ));
    }
    tokio::task::spawn_blocking(move || transcribe_blocking(&state, samples, vad_flag))
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, "server_error", format!("join: {e}")))?
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, "server_error", e.to_string()))
}

/// 阻塞执行：持 recognizer 锁，按 vad_flag 走单段 / 多段。
fn transcribe_blocking(
    state: &AppState,
    samples: Vec<f32>,
    vad_flag: bool,
) -> anyhow::Result<TranscriptionResponse> {
    let recognizer = state.recognizer.lock().expect("recognizer mutex poisoned");

    if !vad_flag {
        let text = recognize(&recognizer, &samples)?;
        return Ok(TranscriptionResponse { text, segments: None });
    }

    // 音频 < 1 s：跳过 VAD，整段直推，segments 退化为单元素。
    if samples.len() < vad::SAMPLE_RATE as usize {
        let text = recognize(&recognizer, &samples)?;
        let end = samples.len() as f64 / vad::SAMPLE_RATE as f64;
        return Ok(TranscriptionResponse {
            text: text.clone(),
            segments: Some(vec![SegmentOut { start: 0.0, end, text }]),
        });
    }

    let vad_model = state
        .args
        .vad_model
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("vad model path not utf-8"))?;
    let segs = vad::segment(vad_model, &samples)?;

    let mut segments = Vec::new();
    for s in &segs {
        let text = recognize_long(&recognizer, &s.samples, state.is_whisper)?;
        if text.is_empty() {
            continue;
        }
        segments.push(SegmentOut { start: s.start, end: s.end, text });
    }
    let text = segments.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join(" ");
    Ok(TranscriptionResponse { text, segments: Some(segments) })
}

fn recognize(recognizer: &OfflineRecognizer, samples: &[f32]) -> anyhow::Result<String> {
    let stream = recognizer.create_stream();
    stream.accept_waveform(vad::SAMPLE_RATE, samples);
    recognizer.decode(&stream);
    let result = stream
        .get_result()
        .ok_or_else(|| anyhow::anyhow!("recognizer returned no result"))?;
    Ok(result.text.trim().to_string())
}

/// 段内超 30 s 且 whisper 模式时按 25 s 窗 + 2 s 重叠（步长 23 s）硬切，
/// 逐窗推理后拼接（重叠区交给后一窗，前窗边界更易吞字）。其它情况整段直推。
fn recognize_long(recognizer: &OfflineRecognizer, samples: &[f32], is_whisper: bool) -> anyhow::Result<String> {
    const SR: usize = vad::SAMPLE_RATE as usize;
    if !is_whisper || samples.len() <= 30 * SR {
        return recognize(recognizer, samples);
    }
    let win = 25 * SR;
    let step = 23 * SR;
    let mut texts = Vec::new();
    let mut i = 0;
    while i < samples.len() {
        let end = (i + win).min(samples.len());
        let t = recognize(recognizer, &samples[i..end])?;
        if !t.is_empty() {
            texts.push(t);
        }
        if end == samples.len() {
            break;
        }
        i += step;
    }
    Ok(texts.join(" "))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn wav_header(channels: u16, sample_rate: u32, bits: u16, audio_format: u16) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&[0, 0, 0, 0]);
        v.extend_from_slice(b"WAVE");
        v.extend_from_slice(b"fmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&audio_format.to_le_bytes());
        v.extend_from_slice(&channels.to_le_bytes());
        v.extend_from_slice(&sample_rate.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes()); // byte rate
        v.extend_from_slice(&0u16.to_le_bytes()); // block align
        v.extend_from_slice(&bits.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&0u32.to_le_bytes());
        v
    }

    #[test]
    fn fast_wav_accepts_16k_mono_pcm16() {
        assert!(is_fast_wav(&wav_header(1, 16000, 16, 1)));
    }

    #[test]
    fn fast_wav_accepts_16k_mono_float32() {
        assert!(is_fast_wav(&wav_header(1, 16000, 32, 3)));
    }

    #[test]
    fn fast_wav_rejects_stereo_and_44k() {
        assert!(!is_fast_wav(&wav_header(2, 16000, 16, 1)));
        assert!(!is_fast_wav(&wav_header(1, 44100, 16, 1)));
        assert!(!is_fast_wav(&wav_header(1, 16000, 24, 1))); // 24-bit → ffmpeg
    }

    #[test]
    fn fast_wav_rejects_non_riff() {
        assert!(!is_fast_wav(b"not an audio file at all........"));
        assert!(!is_fast_wav(&[]));
    }

    #[test]
    fn bool_field_parsing() {
        assert!(parse_bool_field("true"));
        assert!(parse_bool_field("1"));
        assert!(parse_bool_field(" TRUE "));
        assert!(!parse_bool_field("false"));
        assert!(!parse_bool_field("0"));
        assert!(!parse_bool_field(""));
    }

    #[test]
    fn file_path_rejects_percent_encoding() {
        let r = validate_file_path("file:///tmp/%2e%2e/etc/passwd", "/tmp/%2e%2e/etc/passwd");
        let (code, _) = r.unwrap_err();
        assert_eq!(code, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn file_path_rejects_windows_style() {
        assert!(is_windows_style("/C:/Users/x"));
        let r = validate_file_path("file:///C:/Users/x", "/C:/Users/x");
        assert!(r.is_err());
    }

    #[test]
    fn file_path_accepts_posix_abs() {
        let p = validate_file_path("file:///tmp/a.mp4", "/tmp/a.mp4").unwrap();
        assert_eq!(p, PathBuf::from("/tmp/a.mp4"));
    }

    #[test]
    fn allowlist_prefix_check() {
        let allow = vec![PathBuf::from("/home/fengqi/.config/zero/downloads")];
        assert!(path_in_allowlist(
            Path::new("/home/fengqi/.config/zero/downloads/douyin/x.mp4"),
            &allow
        ));
        assert!(!path_in_allowlist(Path::new("/etc/passwd"), &allow));
    }
}
