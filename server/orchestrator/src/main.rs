//! P0 编排层骨架:对客户端的 WebSocket(protocol-draft.md)<-> ASR 服务 + vLLM。
//! 业务胶水,不做推理。标 TODO 处为联调时硬化(重连/背压/并发优化)。

mod protocol;

use std::sync::atomic::{AtomicU64, Ordering};

use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::Response,
    routing::get,
    Router,
};
use futures_util::{SinkExt, StreamExt};
use protocol::{ClientControl, Hello, ServerEvent};
use tokio_tungstenite::tungstenite::Message as TMessage;

#[derive(Clone)]
struct Cfg {
    bind: String,
    asr_ws: String,
    vllm_base: String,
    vllm_model: String,
}

fn cfg() -> Cfg {
    Cfg {
        bind: std::env::var("ORCH_BIND").unwrap_or_else(|_| "0.0.0.0:8090".into()),
        asr_ws: std::env::var("ASR_WS").unwrap_or_else(|_| "ws://asr:9100".into()),
        vllm_base: std::env::var("VLLM_BASE")
            .unwrap_or_else(|_| "http://host.docker.internal:1234/v1".into()),
        vllm_model: std::env::var("VLLM_MODEL").unwrap_or_else(|_| "default".into()),
    }
}

static SEG_ID: AtomicU64 = AtomicU64::new(1);

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let c = cfg();
    let bind = c.bind.clone();
    let app = Router::new().route("/stream", get(ws_upgrade));
    let listener = tokio::net::TcpListener::bind(&bind).await.expect("bind");
    tracing::info!("orchestrator listening on {bind} (asr={})", c.asr_ws);
    axum::serve(listener, app).await.expect("serve");
}

async fn ws_upgrade(ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(handle_client)
}

async fn handle_client(mut sock: WebSocket) {
    let c = cfg();
    let session_id = format!("s{}", SEG_ID.load(Ordering::Relaxed));

    // 1) hello
    let hello: Hello = match sock.recv().await {
        Some(Ok(Message::Text(t))) => match serde_json::from_str(&t) {
            Ok(h) => h,
            Err(e) => return send_fatal(&mut sock, "bad_hello", &e.to_string()).await,
        },
        _ => return,
    };
    let _ = sock
        .send(Message::Text(
            ServerEvent::Ready { session_id: session_id.clone() }.json(),
        ))
        .await;

    // 2) 连接 ASR 服务
    let (asr, _) = match tokio_tungstenite::connect_async(&c.asr_ws).await {
        Ok(x) => x,
        Err(e) => return send_fatal(&mut sock, "asr_unreachable", &e.to_string()).await,
    };

    // 拆分:客户端写端交给 asr_reader(唯一写者),asr 读端并发转发。
    let (cli_tx, mut cli_rx) = sock.split();
    let (mut asr_tx, asr_rx) = asr.split();

    // asr_reader:全程并发读 ASR(流式 VAD 会在过程中持续吐 segment),
    // 转发并按需调 vLLM,收到 done 则发 Done 结束。
    let reader = tokio::spawn(asr_reader(asr_rx, cli_tx, hello, c.clone(), session_id));

    // 主循环:客户端音频/控制 -> ASR
    loop {
        match cli_rx.next().await {
            Some(Ok(Message::Binary(pcm))) => {
                if asr_tx.send(TMessage::Binary(pcm)).await.is_err() {
                    break;
                }
            }
            Some(Ok(Message::Text(t))) => match serde_json::from_str::<ClientControl>(&t) {
                Ok(ClientControl::Reset) => {
                    let _ = asr_tx.send(TMessage::text(r#"{"type":"reset"}"#)).await;
                }
                Ok(ClientControl::Stop) => {
                    let _ = asr_tx.send(TMessage::text(r#"{"type":"flush"}"#)).await;
                    break;
                }
                Err(_) => {}
            },
            _ => {
                // 断线/关闭:让 ASR 收尾,asr_reader 会发 done
                let _ = asr_tx.send(TMessage::text(r#"{"type":"flush"}"#)).await;
                break;
            }
        }
    }

    // 等 asr_reader 处理完 flush 后的收尾(它负责发 Done)
    let _ = tokio::time::timeout(std::time::Duration::from_secs(30), reader).await;
}

/// 并发读取 ASR,逐段转发客户端,并按需调 vLLM 优化/翻译。收到 done 发 Done。
async fn asr_reader(
    mut asr_rx: futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    mut cli_tx: futures_util::stream::SplitSink<WebSocket, Message>,
    hello: Hello,
    c: Cfg,
    session_id: String,
) {
    while let Some(Ok(msg)) = asr_rx.next().await {
        let TMessage::Text(t) = msg else { continue };
        let v: serde_json::Value = match serde_json::from_str(&t) {
            Ok(v) => v,
            Err(_) => continue,
        };
        match v.get("type").and_then(|x| x.as_str()) {
            Some("segment") => {
                let id = SEG_ID.fetch_add(1, Ordering::Relaxed);
                let text = v.get("text").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let _ = cli_tx
                    .send(Message::Text(
                        ServerEvent::Segment {
                            id,
                            text: text.clone(),
                            t_start: v.get("t_start").and_then(|x| x.as_f64()).map(|x| x as f32),
                            t_end: v.get("t_end").and_then(|x| x.as_f64()).map(|x| x as f32),
                        }
                        .json(),
                    ))
                    .await;

                // TODO 并发化:P0 顺序调用(会延后后续段转发);先求对
                if hello.want_optimize {
                    if let Ok(opt) = llm(
                        &c,
                        "你是中文口语转写规整器。把用户这句口语整理成通顺、简洁的书面中文。\
                         严格要求:只输出整理后的一句话本身;不要解释、不要选项、不要列表、\
                         不要markdown、不要追问、不要任何前后缀;若已通顺则原样返回。",
                        &text,
                    )
                    .await
                    {
                        let _ = cli_tx
                            .send(Message::Text(ServerEvent::Optimized { r#ref: id, text: opt }.json()))
                            .await;
                    }
                }
                if hello.want_translate {
                    if let Ok(en) = llm(
                        &c,
                        "Translate the user's sentence into natural English. Output ONLY the \
                         translation itself — no explanations, no options, no quotes, no markdown.",
                        &text,
                    )
                    .await
                    {
                        let _ = cli_tx
                            .send(Message::Text(ServerEvent::Translated { r#ref: id, text: en }.json()))
                            .await;
                    }
                }
            }
            Some("error") => {
                let m = v.get("message").and_then(|x| x.as_str()).unwrap_or("asr error");
                let _ = cli_tx
                    .send(Message::Text(
                        ServerEvent::Error {
                            code: "asr".into(),
                            message: m.into(),
                            fatal: false,
                        }
                        .json(),
                    ))
                    .await;
            }
            Some("done") => {
                let _ = cli_tx
                    .send(Message::Text(ServerEvent::Done { session_id }.json()))
                    .await;
                return;
            }
            _ => {}
        }
    }
}

/// OpenAI 兼容 chat completions(指向主机上的 vLLM)。
async fn llm(c: &Cfg, sys: &str, user: &str) -> anyhow::Result<String> {
    let body = serde_json::json!({
        "model": c.vllm_model,
        "messages": [
            {"role": "system", "content": sys},
            {"role": "user", "content": user}
        ],
        "temperature": 0.2,
        "max_tokens": 256,
        "stream": false
    });
    let resp = reqwest::Client::new()
        .post(format!("{}/chat/completions", c.vllm_base))
        .json(&body)
        .send()
        .await?
        .error_for_status()?
        .json::<serde_json::Value>()
        .await?;
    Ok(resp["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string())
}

async fn send_fatal(sock: &mut WebSocket, code: &str, msg: &str) {
    let _ = sock
        .send(Message::Text(
            ServerEvent::Error { code: code.into(), message: msg.into(), fatal: true }.json(),
        ))
        .await;
}
