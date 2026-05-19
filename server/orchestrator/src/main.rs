//! P0 编排层骨架:对客户端的 WebSocket(protocol-draft.md)<-> ASR 服务 + vLLM。
//! 业务胶水,不做推理。标 TODO 处为联调时硬化(重连/背压/并发优化)。

mod db;
mod protocol;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use axum::{
    body::Bytes,
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    extract::{Path, Query, State},
    response::{Html, Response},
    routing::{delete, get, post},
    Json, Router,
};
use std::collections::HashMap;
use db::Db;
use futures_util::{SinkExt, StreamExt};
use protocol::{ClientControl, Hello, ServerEvent};
use serde_json::json;
use tokio_tungstenite::tungstenite::Message as TMessage;

#[derive(Clone)]
struct Cfg {
    bind: String,
    asr_ws: String,
    asr_embed: String,
    vllm_base: String,
    vllm_model: String,
}

#[derive(Clone)]
struct AppCtx {
    cfg: Cfg,
    db: Arc<Db>,
}

fn cfg() -> Cfg {
    Cfg {
        bind: std::env::var("ORCH_BIND").unwrap_or_else(|_| "0.0.0.0:8090".into()),
        asr_ws: std::env::var("ASR_WS").unwrap_or_else(|_| "ws://asr:9100".into()),
        asr_embed: std::env::var("ASR_EMBED")
            .unwrap_or_else(|_| "http://asr:9101/embed".into()),
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

    let data_dir = std::env::var("DATA_DIR").unwrap_or_else(|_| "/data".into());
    let _ = std::fs::create_dir_all(&data_dir);
    let db = Arc::new(
        Db::open(&format!("{data_dir}/app.db")).expect("open app.db"),
    );
    let ctx = AppCtx { cfg: c.clone(), db };

    let app = Router::new()
        .route("/stream", get(ws_upgrade))
        .route("/", get(console))
        .route("/api/stats", get(api_stats))
        .route("/api/history", get(api_history))
        .route("/api/speakers", get(api_speakers))
        .route("/api/speakers/enroll", post(api_speaker_enroll))
        .route("/api/voiceprints", get(api_voiceprints))
        .route("/api/speakers/:id", delete(api_speaker_delete))
        .route("/api/speakers/:id/rename", post(api_speaker_rename))
        .route("/api/speakers/:id/enabled", post(api_speaker_enabled))
        .route("/api/config", get(api_config_get).post(api_config_set))
        .with_state(ctx);

    let listener = tokio::net::TcpListener::bind(&bind).await.expect("bind");
    tracing::info!("orchestrator listening on {bind} (asr={})", c.asr_ws);
    axum::serve(listener, app).await.expect("serve");
}

async fn ws_upgrade(State(ctx): State<AppCtx>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |s| handle_client(s, ctx))
}

async fn handle_client(mut sock: WebSocket, ctx: AppCtx) {
    let c = ctx.cfg.clone();
    let db = ctx.db.clone();
    let session_id = format!("s{}", SEG_ID.load(Ordering::Relaxed));
    db.session_start(&session_id);
    let started = Instant::now();

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
    let reader = tokio::spawn(asr_reader(
        asr_rx,
        cli_tx,
        hello,
        c.clone(),
        session_id.clone(),
        db.clone(),
    ));

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
    db.session_end(&session_id, started.elapsed().as_secs_f64());
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
    db: Arc<Db>,
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
                let t0 = v.get("t_start").and_then(|x| x.as_f64()).unwrap_or(0.0);
                let t1 = v.get("t_end").and_then(|x| x.as_f64()).unwrap_or(0.0);
                let speaker = v.get("speaker").and_then(|x| x.as_str());
                db.segment_upsert(id as i64, &session_id, &text, None, None, t0, t1, speaker);
                let _ = cli_tx
                    .send(Message::Text(
                        ServerEvent::Segment {
                            id,
                            text: text.clone(),
                            t_start: Some(t0 as f32),
                            t_end: Some(t1 as f32),
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
                        db.segment_set_optimized(id as i64, &opt);
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
                        db.segment_set_english(id as i64, &en);
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

// ── Web 管理台 HTTP API ──────────────────────────────────────────────────

async fn console() -> Html<&'static str> {
    Html(CONSOLE_HTML)
}
async fn api_stats(State(ctx): State<AppCtx>) -> Json<db::Stats> {
    Json(ctx.db.stats())
}
async fn api_history(State(ctx): State<AppCtx>) -> Json<Vec<db::SegmentRow>> {
    Json(ctx.db.segments_recent(200))
}
async fn api_speakers(State(ctx): State<AppCtx>) -> Json<Vec<db::Speaker>> {
    Json(ctx.db.speakers_list())
}
/// Enabled voiceprints for the asr service to pull (gating source of truth).
async fn api_voiceprints(State(ctx): State<AppCtx>) -> Json<serde_json::Value> {
    let vps: Vec<serde_json::Value> = ctx
        .db
        .enabled_voiceprints()
        .into_iter()
        .map(|(name, emb)| json!({ "name": name, "embedding": emb }))
        .collect();
    Json(serde_json::Value::Array(vps))
}

/// Enroll: `?name=` + raw audio body -> asr /embed -> store voiceprint.
async fn api_speaker_enroll(
    State(ctx): State<AppCtx>,
    Query(q): Query<HashMap<String, String>>,
    body: Bytes,
) -> Json<serde_json::Value> {
    let name = q.get("name").cloned().unwrap_or_default();
    if name.trim().is_empty() {
        return Json(json!({"ok": false, "error": "缺少名称"}));
    }
    let resp = reqwest::Client::new()
        .post(&ctx.cfg.asr_embed)
        .body(body.to_vec())
        .send()
        .await;
    let j = match resp {
        Ok(r) => match r.json::<serde_json::Value>().await {
            Ok(j) => j,
            Err(e) => return Json(json!({"ok": false, "error": format!("embed 解析失败: {e}")})),
        },
        Err(e) => return Json(json!({"ok": false, "error": format!("asr 不可达: {e}")})),
    };
    let emb: Vec<f32> = match j.get("embedding").and_then(|x| x.as_array()) {
        Some(a) => a.iter().filter_map(|x| x.as_f64()).map(|x| x as f32).collect(),
        None => {
            let e = j.get("error").and_then(|x| x.as_str()).unwrap_or("embed 失败");
            return Json(json!({"ok": false, "error": e}));
        }
    };
    if emb.is_empty() {
        return Json(json!({"ok": false, "error": "空声纹向量"}));
    }
    let csv = emb.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(",");
    match ctx.db.speaker_add(&name, &csv) {
        Ok(id) => Json(json!({"ok": true, "id": id})),
        Err(e) => Json(json!({"ok": false, "error": format!("保存失败(名称重复?): {e}")})),
    }
}

async fn api_speaker_delete(
    State(ctx): State<AppCtx>,
    Path(id): Path<i64>,
) -> Json<serde_json::Value> {
    ctx.db.speaker_delete(id);
    Json(json!({"ok": true}))
}
async fn api_speaker_rename(
    State(ctx): State<AppCtx>,
    Path(id): Path<i64>,
    Json(b): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    if let Some(n) = b.get("name").and_then(|x| x.as_str()) {
        ctx.db.speaker_rename(id, n);
    }
    Json(json!({"ok": true}))
}
async fn api_speaker_enabled(
    State(ctx): State<AppCtx>,
    Path(id): Path<i64>,
    Json(b): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    let e = b.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true);
    ctx.db.speaker_set_enabled(id, e);
    Json(json!({"ok": true}))
}
async fn api_config_get(State(ctx): State<AppCtx>) -> Json<serde_json::Value> {
    let m: serde_json::Map<String, serde_json::Value> = ctx
        .db
        .config_all()
        .into_iter()
        .map(|(k, v)| (k, serde_json::Value::String(v)))
        .collect();
    Json(serde_json::Value::Object(m))
}
async fn api_config_set(
    State(ctx): State<AppCtx>,
    Json(b): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    if let Some(o) = b.as_object() {
        for (k, v) in o {
            if let Some(s) = v.as_str() {
                ctx.db.config_set(k, s);
            }
        }
    }
    Json(json!({"ok": true}))
}

const CONSOLE_HTML: &str = r#"<!doctype html><html lang="zh"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>语音服务管理台</title><style>
body{font-family:system-ui,Segoe UI,sans-serif;margin:0;background:#0f1115;color:#e6e6e6}
header{padding:14px 20px;background:#171a21;font-size:18px;font-weight:600}
nav{display:flex;gap:6px;padding:10px 20px;background:#13161c}
nav button{background:#222733;color:#cbd5e1;border:0;padding:8px 14px;border-radius:8px;cursor:pointer}
nav button.on{background:#2563eb;color:#fff}
main{padding:20px;max-width:1000px}
.card{background:#171a21;border:1px solid #232838;border-radius:12px;padding:16px;margin-bottom:14px}
table{width:100%;border-collapse:collapse;font-size:14px}
td,th{padding:8px;border-bottom:1px solid #232838;text-align:left;vertical-align:top}
button.s{padding:4px 10px;border-radius:6px;border:0;cursor:pointer;font-size:12px}
.del{background:#7f1d1d;color:#fff}.ren{background:#334155;color:#fff}
.kpi{display:flex;gap:24px}.kpi div{font-size:13px;color:#94a3b8}.kpi b{display:block;font-size:24px;color:#fff}
.note{color:#94a3b8;font-size:13px}
input{background:#0f1115;border:1px solid #334155;color:#e6e6e6;border-radius:6px;padding:6px}
</style></head><body>
<header>语音服务管理台</header>
<nav><button class=on data-t=ov>概览</button><button data-t=hi>历史</button>
<button data-t=sp>声纹</button><button data-t=cf>配置</button></nav>
<main><div id=v></div></main>
<script>
const V=document.getElementById('v');let tab='ov';
document.querySelectorAll('nav button').forEach(b=>b.onclick=()=>{
 document.querySelectorAll('nav button').forEach(x=>x.classList.remove('on'));
 b.classList.add('on');tab=b.dataset.t;render()});
async function j(u,m,bd){const o={method:m||'GET'};if(bd){o.headers={'content-type':'application/json'};o.body=JSON.stringify(bd)}return (await fetch(u,o)).json()}
async function render(){
 if(tab=='ov'){const s=await j('/api/stats');
  V.innerHTML=`<div class=card><div class=kpi>
  <div>会话数<b>${s.sessions}</b></div><div>识别段<b>${s.segments}</b></div>
  <div>累计录音<b>${(s.total_recording_sec/60).toFixed(1)}分</b></div>
  <div>今日录音<b>${(s.today_recording_sec/60).toFixed(1)}分</b></div></div></div>`}
 else if(tab=='hi'){const h=await j('/api/history');
  V.innerHTML='<div class=card><table><tr><th>时间</th><th>原文</th><th>优化</th><th>英文</th><th>说话人</th></tr>'+
  h.map(r=>`<tr><td>${r.ts}</td><td>${esc(r.text)}</td><td>${esc(r.optimized||'')}</td><td>${esc(r.english||'')}</td><td>${esc(r.speaker||'')}</td></tr>`).join('')+'</table></div>'}
 else if(tab=='sp'){const sp=await j('/api/speakers');
  V.innerHTML='<div class=card><p class=note>注册:点"录制注册"→对麦克风清晰说约5秒。仅"启用"的声纹参与门控:命中才识别,其余丢弃。</p>'+
  '<p><button class="s ren" onclick="enroll()">● 录制注册</button> <span id=est></span></p>'+
  '<table><tr><th>名称</th><th>启用</th><th>创建</th><th></th></tr>'+
  sp.map(s=>`<tr><td>${esc(s.name)}</td><td><input type=checkbox ${s.enabled?'checked':''} onchange="en(${s.id},this.checked)"></td><td>${s.created_at}</td>
  <td><button class="s ren" onclick="rn(${s.id})">改名</button> <button class="s del" onclick="dl(${s.id})">删除</button></td></tr>`).join('')+'</table></div>'}
 else if(tab=='cf'){const c=await j('/api/config');const ks=Object.keys(c);
  V.innerHTML='<div class=card><table>'+ks.map(k=>`<tr><td>${esc(k)}</td><td><input id="cf_${k}" value="${esc(c[k])}" style="width:60%"></td><td><button class="s ren" onclick="cs('${k}')">保存</button></td></tr>`).join('')+
  (ks.length?'':'<tr><td class=note>暂无配置项</td></tr>')+'</table></div>'}
}
function esc(s){return (s+'').replace(/[&<>]/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;'}[c]))}
async function dl(id){if(confirm('删除该声纹?')){await j('/api/speakers/'+id,'DELETE');render()}}
async function rn(id){const n=prompt('新名称');if(n){await j('/api/speakers/'+id+'/rename','POST',{name:n});render()}}
async function en(id,e){await j('/api/speakers/'+id+'/enabled','POST',{enabled:e})}
async function cs(k){const val=document.getElementById('cf_'+k).value;await j('/api/config','POST',{[k]:val});alert('已保存')}
async function enroll(){
 const name=prompt('声纹名称(如:张三)');if(!name)return;
 const est=document.getElementById('est');
 let stream;try{stream=await navigator.mediaDevices.getUserMedia({audio:true})}catch(e){alert('无法访问麦克风:'+e);return}
 const mr=new MediaRecorder(stream);const chunks=[];
 mr.ondataavailable=e=>chunks.push(e.data);
 mr.onstop=async()=>{
  stream.getTracks().forEach(t=>t.stop());est.textContent='上传中...';
  const blob=new Blob(chunks);
  const r=await fetch('/api/speakers/enroll?name='+encodeURIComponent(name),{method:'POST',body:blob});
  const d=await r.json();est.textContent='';
  if(d.ok){alert('注册成功');render()}else{alert('注册失败:'+(d.error||'?'))}
 };
 mr.start();est.textContent='录音中…(5秒)';setTimeout(()=>mr.stop(),5000);
}
render();
</script></body></html>"#;

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
