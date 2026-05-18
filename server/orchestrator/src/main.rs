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
    let (mut asr, _) = match tokio_tungstenite::connect_async(&c.asr_ws).await {
        Ok(x) => x,
        Err(e) => return send_fatal(&mut sock, "asr_unreachable", &e.to_string()).await,
    };

    // 3) 转发循环:客户端音频 -> ASR;stop -> flush 并回收段
    loop {
        match sock.recv().await {
            Some(Ok(Message::Binary(pcm))) => {
                if asr.send(TMessage::Binary(pcm)).await.is_err() {
                    break;
                }
            }
            Some(Ok(Message::Text(t))) => {
                match serde_json::from_str::<ClientControl>(&t) {
                    Ok(ClientControl::Reset) => {
                        let _ = asr.send(TMessage::text(r#"{"type":"reset"}"#)).await;
                    }
                    Ok(ClientControl::Stop) => {
                        let _ = asr.send(TMessage::text(r#"{"type":"flush"}"#)).await;
                        drain_asr(&mut asr, &mut sock, &hello, &c).await;
                        break;
                    }
                    Err(_) => {}
                }
            }
            _ => break, // 断线/关闭:P0 视为会话结束
        }
    }

    let _ = sock
        .send(Message::Text(ServerEvent::Done { session_id }.json()))
        .await;
}

/// flush 后从 ASR 读段,逐段回客户端,并按需调 vLLM 优化/翻译。
async fn drain_asr(
    asr: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    sock: &mut WebSocket,
    hello: &Hello,
    c: &Cfg,
) {
    while let Some(Ok(msg)) = asr.next().await {
        let TMessage::Text(t) = msg else { continue };
        let v: serde_json::Value = match serde_json::from_str(&t) {
            Ok(v) => v,
            Err(_) => continue,
        };
        match v.get("type").and_then(|x| x.as_str()) {
            Some("segment") => {
                let id = SEG_ID.fetch_add(1, Ordering::Relaxed);
                let text = v.get("text").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let _ = sock
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

                // TODO 并发化:P0 顺序调用,够用先求对
                if hello.want_optimize {
                    if let Ok(opt) = llm(c, "请在不改变原意的前提下润色为通顺中文,只输出结果:", &text).await {
                        let _ = sock
                            .send(Message::Text(ServerEvent::Optimized { r#ref: id, text: opt }.json()))
                            .await;
                    }
                }
                if hello.want_translate {
                    if let Ok(en) = llm(c, "Translate to natural English, output only the translation:", &text).await {
                        let _ = sock
                            .send(Message::Text(ServerEvent::Translated { r#ref: id, text: en }.json()))
                            .await;
                    }
                }
            }
            Some("done") => break,
            Some("error") => {
                let m = v.get("message").and_then(|x| x.as_str()).unwrap_or("asr error");
                let _ = sock
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
        "temperature": 0.3,
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
