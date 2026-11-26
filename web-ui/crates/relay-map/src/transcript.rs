//! Session logs for the agent panel: find an agent's full transcript, read it
//! from this computer, over ssh from the computer that has it, or from
//! VictoriaLogs, and flatten it into readable turns.
//!
//! Only the transcript path a collector reported for the agent is ever read,
//! never a path from the request.

use crate::collect::common::{content_text, parse_ts, s};
use crate::hub::Shared;
use anyhow::{anyhow, bail, Context, Result};
use relay_map_core::{Agent, AgentKind, TranscriptEntry, TranscriptView};
use serde_json::Value;
use std::process::Stdio;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::process::Command;

/// The readable view reads at most this much from the end of a log.
pub const VIEW_BYTES: u64 = 12 << 20;
/// A raw download stops here.
pub const RAW_BYTES: u64 = 512 << 20;
/// Longest text kept for one entry in the readable view.
const ENTRY_CHARS: usize = 6_000;
/// Most entries returned in the readable view (the latest ones).
const MAX_ENTRIES: usize = 4_000;

/// Where an agent's log can be read from, worked out under the hub lock.
pub struct Plan {
    pub agent: Agent,
    /// Read the file on this computer.
    pub local: bool,
    /// ssh target of the computer that has the file.
    pub ssh: Option<String>,
    pub victorialogs: Option<String>,
}

pub fn plan(hub: &Shared, addr: &str) -> Result<Plan> {
    let h = hub.lock().unwrap();
    let agent = h.agents.get(addr).cloned().ok_or_else(|| anyhow!("no agent {addr}"))?;
    let local = h.cfg.local && agent.host == h.self_host;
    // Only computers named in the config are reached over ssh.
    let ssh = h.cfg.hosts.iter().find(|c| c.name == agent.host).map(|c| c.ssh_target().to_string());
    Ok(Plan { agent, local, ssh, victorialogs: h.cfg.victorialogs_url.clone() })
}

/// The raw bytes of the log (or its last `max` bytes) and the full size, trying
/// this computer, then ssh, then VictoriaLogs.
pub async fn fetch(p: &Plan, max: u64) -> Result<(Vec<u8>, u64, &'static str, Vec<String>)> {
    let mut notes = Vec::new();
    if let Some(path) = p.agent.transcript.as_deref() {
        if p.local {
            match read_local(path, max).await {
                Ok((b, n)) => return Ok((b, n, "local", notes)),
                Err(e) => notes.push(format!("this computer: {e:#}")),
            }
        }
        if let Some(t) = &p.ssh {
            match read_ssh(t, path, max).await {
                Ok((b, n)) => return Ok((b, n, "ssh", notes)),
                Err(e) => notes.push(format!("ssh {t}: {e:#}")),
            }
        } else if !p.local {
            notes.push(format!("{} is not a configured ssh host", p.agent.host));
        }
    } else {
        notes.push("the collector reported no transcript path".into());
    }
    match &p.victorialogs {
        Some(url) => match read_victorialogs(url, &p.agent, max).await {
            Ok(b) if !b.is_empty() => {
                let n = b.len() as u64;
                Ok((b, n, "victorialogs", notes))
            }
            Ok(_) => {
                notes.push(format!("VictoriaLogs has no lines for session {}", p.agent.id));
                bail!("{}", notes.join("; "))
            }
            Err(e) => {
                notes.push(format!("VictoriaLogs: {e:#}"));
                bail!("{}", notes.join("; "))
            }
        },
        None => {
            notes.push("no victorialogs_url configured".into());
            bail!("{}", notes.join("; "))
        }
    }
}

async fn read_local(path: &str, max: u64) -> Result<(Vec<u8>, u64)> {
    let mut f = tokio::fs::File::open(path).await.with_context(|| format!("opening {path}"))?;
    let len = f.metadata().await?.len();
    if len > max {
        f.seek(std::io::SeekFrom::Start(len - max)).await?;
    }
    let mut buf = Vec::with_capacity(len.min(max) as usize);
    f.take(max).read_to_end(&mut buf).await?;
    Ok((buf, len))
}

async fn read_ssh(target: &str, path: &str, max: u64) -> Result<(Vec<u8>, u64)> {
    let q = shell_words::quote(path);
    // First line: the file size; the rest: the bytes.
    let cmd = format!("wc -c < {q} && tail -c {max} -- {q}");
    let out = Command::new("ssh")
        .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=10", target, &cmd])
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    let nl = out.stdout.iter().position(|b| *b == b'\n').ok_or_else(|| anyhow!("no size line"))?;
    let len: u64 = String::from_utf8_lossy(&out.stdout[..nl]).trim().parse().context("size line")?;
    Ok((out.stdout[nl + 1..].to_vec(), len))
}

/// Log lines whose `session_id` field is the agent's id, oldest first, one
/// original JSON line each (the `_msg` field), as a log shipper would store them.
async fn read_victorialogs(url: &str, a: &Agent, max: u64) -> Result<Vec<u8>> {
    let query = format!("_time:30d session_id:={} | sort by (_time) | fields _msg", logsql_quote(&a.id));
    let endpoint = format!("{}/select/logsql/query", url.trim_end_matches('/'));
    let out = Command::new("curl")
        .args(["-fsS", "--max-time", "30", "--data-urlencode"])
        .arg(format!("query={query}"))
        .arg(&endpoint)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    let mut buf = Vec::new();
    for line in out.stdout.split(|b| *b == b'\n') {
        let Ok(v) = serde_json::from_slice::<Value>(line) else { continue };
        if let Some(m) = v.get("_msg").and_then(|x| x.as_str()) {
            buf.extend_from_slice(m.as_bytes());
            buf.push(b'\n');
        }
    }
    if buf.len() as u64 > max {
        buf.drain(..buf.len() - max as usize);
    }
    Ok(buf)
}

fn logsql_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Flatten raw JSONL into readable turns.
pub fn view(a: &Agent, raw: &[u8], bytes: u64, source: &str, notes: Vec<String>) -> TranscriptView {
    let mut entries = Vec::new();
    let mut skipped = 0;
    let mut lines = raw.split(|b| *b == b'\n');
    // A tail read starts mid-line; drop the partial first line.
    if (raw.len() as u64) < bytes {
        lines.next();
    }
    for line in lines {
        if line.iter().all(|b| b.is_ascii_whitespace()) {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            skipped += 1;
            continue;
        };
        let before = entries.len();
        match a.kind {
            Some(AgentKind::Claude) => claude(&v, &mut entries),
            Some(AgentKind::Codex) => codex(&v, &mut entries),
            Some(AgentKind::Antigravity) => agy(&v, &mut entries),
            Some(AgentKind::Pi) => pi(&v, &mut entries),
            _ => generic(&v, &mut entries),
        }
        if entries.len() == before {
            skipped += 1;
        }
    }
    if entries.len() > MAX_ENTRIES {
        entries.drain(..entries.len() - MAX_ENTRIES);
    }
    TranscriptView {
        addr: a.addr.clone(),
        host: a.host.clone(),
        path: a.transcript.clone(),
        source: source.into(),
        bytes,
        shown_bytes: raw.len() as u64,
        skipped,
        entries,
        notes,
    }
}

fn push(out: &mut Vec<TranscriptEntry>, t: i64, role: &str, name: Option<&str>, text: &str) {
    let text = text.trim();
    if text.is_empty() && name.is_none() {
        return;
    }
    let text = if text.chars().count() > ENTRY_CHARS {
        let cut: String = text.chars().take(ENTRY_CHARS).collect();
        format!("{cut}\n… ({} more characters)", text.chars().count() - ENTRY_CHARS)
    } else {
        text.to_string()
    };
    out.push(TranscriptEntry { t, role: role.into(), name: name.map(String::from), text });
}

fn pretty(v: &Value) -> String {
    match v {
        Value::String(x) => x.clone(),
        Value::Null => String::new(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    }
}

/// The most telling argument of a tool call, else the whole input.
fn tool_text(input: &Value) -> String {
    for k in ["command", "cmd", "CommandLine", "file_path", "path", "pattern", "query", "url", "prompt", "description"] {
        if let Some(x) = input.get(k).and_then(|x| x.as_str()) {
            let rest = pretty(input);
            return if rest.len() > x.len() + 40 { format!("{x}\n\n{rest}") } else { x.to_string() };
        }
    }
    pretty(input)
}

fn result_text(c: &Value) -> String {
    match c {
        Value::Array(items) => items
            .iter()
            .map(|i| match i.get("type").and_then(s) {
                Some("text") => i.get("text").and_then(s).unwrap_or("").to_string(),
                Some("image") => "[image]".into(),
                _ => pretty(i),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        other => pretty(other),
    }
}

fn claude(v: &Value, out: &mut Vec<TranscriptEntry>) {
    let t = v.get("timestamp").and_then(parse_ts).unwrap_or(0);
    let ty = v.get("type").and_then(s).unwrap_or("");
    match ty {
        "user" | "assistant" => {
            let Some(m) = v.get("message") else { return };
            match m.get("content") {
                Some(Value::String(x)) => push(out, t, ty, None, x),
                Some(Value::Array(items)) => {
                    for it in items {
                        match it.get("type").and_then(s) {
                            Some("text") => push(out, t, ty, None, it.get("text").and_then(s).unwrap_or("")),
                            Some("thinking") => push(out, t, "thinking", None, it.get("thinking").and_then(s).unwrap_or("")),
                            Some("tool_use") => {
                                let name = it.get("name").and_then(s).unwrap_or("tool");
                                push(out, t, "tool", Some(name), &tool_text(it.get("input").unwrap_or(&Value::Null)));
                            }
                            Some("tool_result") => {
                                push(out, t, "result", Some("result"), &result_text(it.get("content").unwrap_or(&Value::Null)));
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        "system" => push(out, t, "system", None, v.get("content").and_then(s).unwrap_or("")),
        "summary" => push(out, t, "system", Some("summary"), v.get("summary").and_then(s).unwrap_or("")),
        _ => {}
    }
}

fn codex(v: &Value, out: &mut Vec<TranscriptEntry>) {
    let t = v.get("timestamp").and_then(parse_ts).unwrap_or(0);
    let p = v.get("payload").unwrap_or(&Value::Null);
    let pty = p.get("type").and_then(s).unwrap_or("");
    match (v.get("type").and_then(s).unwrap_or(""), pty) {
        ("response_item", "message") => {
            let role = match p.get("role").and_then(s) {
                Some("assistant") => "assistant",
                Some("user") => "user",
                _ => "system",
            };
            push(out, t, role, None, &content_text(p.get("content").unwrap_or(&Value::Null)));
        }
        ("response_item", "reasoning") => {
            let text = p.get("summary").map(content_text).unwrap_or_default();
            push(out, t, "thinking", None, &text);
        }
        ("response_item", "function_call") | ("response_item", "custom_tool_call") => {
            let name = p.get("name").and_then(s).unwrap_or("tool");
            let args = p.get("arguments").or(p.get("input")).cloned().unwrap_or(Value::Null);
            let args = match &args {
                Value::String(x) => serde_json::from_str::<Value>(x).unwrap_or(args.clone()),
                _ => args,
            };
            push(out, t, "tool", Some(name), &tool_text(&args));
        }
        ("response_item", "function_call_output") | ("response_item", "custom_tool_call_output") => {
            let o = p.get("output").cloned().unwrap_or(Value::Null);
            let o = match &o {
                Value::String(x) => serde_json::from_str::<Value>(x).ok().and_then(|j| j.get("output").cloned()).unwrap_or(o.clone()),
                _ => o,
            };
            push(out, t, "result", Some("result"), &pretty(&o));
        }
        // The newer rollout format records turns as completed items instead.
        ("event_msg", "item_completed") => {
            let item = p.get("item").unwrap_or(&Value::Null);
            let text = item.get("content").map(content_text).or_else(|| item.get("text").and_then(s).map(String::from)).unwrap_or_default();
            match item.get("type").and_then(s) {
                Some("UserMessage") => push(out, t, "user", None, &text),
                Some("AgentMessage") => push(out, t, "assistant", None, &text),
                _ => {}
            }
        }
        _ => {}
    }
}

fn agy(v: &Value, out: &mut Vec<TranscriptEntry>) {
    let t = v.get("created_at").and_then(parse_ts).unwrap_or(0);
    let content = v.get("content").and_then(s).unwrap_or("");
    match v.get("type").and_then(s).unwrap_or("") {
        "USER_INPUT" => push(out, t, "user", None, content),
        "SYSTEM_MESSAGE" => push(out, t, "system", None, content),
        "PLANNER_RESPONSE" => {
            push(out, t, "assistant", None, content);
            for c in v.get("tool_calls").and_then(|x| x.as_array()).into_iter().flatten() {
                let name = c.get("name").and_then(s).unwrap_or("tool");
                push(out, t, "tool", Some(name), &tool_text(c.get("args").unwrap_or(&Value::Null)));
            }
        }
        other if !content.is_empty() => push(out, t, "result", Some(&other.to_lowercase()), content),
        _ => {}
    }
}

fn pi(v: &Value, out: &mut Vec<TranscriptEntry>) {
    let t = v.get("timestamp").and_then(parse_ts).unwrap_or(0);
    if v.get("type").and_then(s) != Some("message") {
        return;
    }
    let Some(m) = v.get("message") else { return };
    let role = m.get("role").and_then(s).unwrap_or("system");
    match m.get("content") {
        Some(Value::Array(items)) => {
            for it in items {
                match it.get("type").and_then(s) {
                    Some("text") => push(out, t, if role == "toolResult" { "result" } else { role }, None, it.get("text").and_then(s).unwrap_or("")),
                    Some("thinking") => push(out, t, "thinking", None, it.get("thinking").and_then(s).unwrap_or("")),
                    Some("toolCall") => {
                        let name = it.get("name").and_then(s).unwrap_or("tool");
                        push(out, t, "tool", Some(name), &tool_text(it.get("arguments").unwrap_or(&Value::Null)));
                    }
                    _ => {}
                }
            }
        }
        Some(c) => push(out, t, role, None, &content_text(c)),
        None => {}
    }
}

fn generic(v: &Value, out: &mut Vec<TranscriptEntry>) {
    let t = v.get("timestamp").and_then(parse_ts).unwrap_or(0);
    push(out, t, "system", None, &pretty(v));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(kind: AgentKind) -> Agent {
        Agent { addr: "x".into(), kind: Some(kind), host: "laptop".into(), ..Default::default() }
    }

    #[test]
    fn claude_turns() {
        let raw = br#"{"type":"user","timestamp":"2026-01-01T00:00:00Z","message":{"role":"user","content":"hello"}}
{"type":"assistant","timestamp":"2026-01-01T00:00:01Z","message":{"content":[{"type":"text","text":"hi"},{"type":"tool_use","name":"Bash","input":{"command":"ls"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","content":[{"type":"text","text":"a.txt"}]}]}}
not json
"#;
        let v = view(&agent(AgentKind::Claude), raw, raw.len() as u64, "local", vec![]);
        let roles: Vec<&str> = v.entries.iter().map(|e| e.role.as_str()).collect();
        assert_eq!(roles, ["user", "assistant", "tool", "result"]);
        assert_eq!(v.entries[2].text, "ls");
        assert_eq!(v.skipped, 1);
    }

    #[test]
    fn tail_drops_partial_line() {
        let raw = br#"t":"x"}
{"type":"user","message":{"content":"second"}}
"#;
        let v = view(&agent(AgentKind::Claude), raw, 10_000, "ssh", vec![]);
        assert_eq!(v.entries.len(), 1);
        assert_eq!(v.skipped, 0);
    }

    #[test]
    fn codex_turns() {
        let raw = br#"{"timestamp":"2026-01-01T00:00:00Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"fix it"}]}}
{"type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":[\"ls\"]}"}}
{"type":"response_item","payload":{"type":"function_call_output","output":"{\"output\":\"done\"}"}}
"#;
        let v = view(&agent(AgentKind::Codex), raw, raw.len() as u64, "local", vec![]);
        let roles: Vec<&str> = v.entries.iter().map(|e| e.role.as_str()).collect();
        assert_eq!(roles, ["user", "tool", "result"]);
        assert_eq!(v.entries[2].text, "done");
    }
}
