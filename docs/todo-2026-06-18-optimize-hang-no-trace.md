# TODO: 中文优化偶发卡死(客户端永久"优化中…")+ trace 缺优化记录

> 报告来源:zero-desktop 语音识别页。现象由下游(toolkit/zero-desktop)观察到,
> 但根因在本仓 orchestrator,故在此记 TODO。日期 2026-06-18。

## 现象

1. 偶尔某一段识别的"中文表达优化"一直转圈(客户端显示"优化中…"),**永远不出结果**,
   也不报错。重启录音才恢复。
2. trace-hub 里**缺这一次中文优化的 LLM 调用记录**(成功的有,卡死的那次没有)。

下游(zero-desktop)只是被动接收 WebSocket 的 `optimized` 事件:收到才把该段标成成功,
否则一直 `running`。下游本身**没有超时**,所以只要本仓不发 `optimized`、也不发 `error`,
该段就永久转圈。下游会另行补一个客户端超时兜底,但**治本在这里**。

## 根因(已定位到行)

优化 / 翻译在 `Some("segment")` 分支里 detached spawn 并发跑
([server/orchestrator/src/main.rs:618](server/orchestrator/src/main.rs:618) 起):

```rust
// main.rs:620-632
let opt_fut = async {
    match &opt_sys {
        Some(s) => llm(&base, &model, s, &opt_user, Some(&llm_ctx)).await.ok(), // ← .ok() 吞错
        None => None,
    }
};
...
let (opt, en) = tokio::join!(opt_fut, tr_fut);
if let Some(opt) = opt { ... send(Optimized) }   // 只有 Some 才回发；None 时什么都不发
```

底层 `llm()`([server/orchestrator/src/main.rs:1426](server/orchestrator/src/main.rs:1426))用
`reqwest::Client::new()` 发请求,**没有设置任何超时**([main.rs:1447](server/orchestrator/src/main.rs:1447)):

```rust
let raw = reqwest::Client::new()
    .post(format!("{}/chat/completions", base))
    .json(&body).send().await? ... .text().await?;
```

两条失效路径:

- **路径 A(卡死,对应现象 1+2)**:vLLM 连上但迟迟不返回 → `.send()/.text()` 的 future
  **永不 resolve** → `llm()` 永远 pending → 既不会发 `Optimized`(客户端永久"优化中"),
  也**到不了** `record_llm_call`(成功/失败两个分支都在请求返回之后,见 [main.rs:1464](server/orchestrator/src/main.rs:1464))
  → **trace 里没有这次记录**。这一条同时解释了两个现象。
- **路径 B(返回错误)**:`llm()` 返回 `Err`(trace 这时会记 Err 分支),但调用处 `.ok()` 把它
  吞成 `None` → 仍然不发 `Optimized`、也不发任何 `error` 事件 → 客户端照样永久转圈。

## 建议修复(三选/可全做)

1. **给 `llm()` 加超时**(治本核心)。`reqwest::Client::builder().timeout(...)` 或外层
   `tokio::time::timeout` 包住请求。超时后变成 `Err`,既能进 trace 的失败分支,也能让上层据此
   通知客户端。建议超时取宽松值(优化是短输出,`max_tokens=256`,十几秒足够)。

2. **优化/翻译失败时给客户端一个终态信号**(止住转圈)。当前只有 `.ok()`。两种做法择一:
   - 发一个针对该 `ref` 的失败事件(客户端已能处理 `type:"error"` 消息,见下游
     `remote.rs` 的 `Some("error")` 分支),让该段从"优化中"切到"失败/可重试";或
   - 失败时把**原文**当作 `Optimized` 文本回发作兜底(保证段落能落定,不卡 UI)。

3. **确保超时/失败一定写 trace**。把 `record_llm_call` 的 Err/timeout 记录提到能覆盖"超时"
   的位置(若用外层 `tokio::time::timeout`,在超时分支也补一条 `SpanStatus` 失败/超时的记录),
   这样 trace-hub 不再出现"卡死的那次完全没记录"。

## 验收

- 人为让 vLLM 卡住(或指向一个不响应的 base)录一段:**客户端不应永久转圈**,应在超时后转入
  失败/兜底,且 **trace-hub 能看到该次优化的失败/超时记录**。
- 正常路径回归:`optimized`/`translated` 正常下发,trace 正常记成功。

## 关联

- 下游侧(zero-desktop)将补"客户端优化超时兜底",与本仓修复互为双保险。
- trace 约定见 [docs/tracing-design.md](docs/tracing-design.md)。
