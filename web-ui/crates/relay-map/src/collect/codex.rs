//! Codex rollouts: `~/.codex/sessions/YYYY/MM/DD/rollout-<time>-<thread id>.jsonl`.

use super::common::*;
use regex::Regex;
use relay_map_core::*;
use serde_json::Value;
use std::sync::LazyLock;

pub fn line(acc: &mut Acc, v: &Value) {
    let ts = v.get("timestamp").and_then(parse_ts);
    let t = ts.unwrap_or(acc.agent.last_active_ms);
    let ty = v.get("type").and_then(s).unwrap_or("");
    let p = v.get("payload").unwrap_or(&Value::Null);
    let pty = p.get("type").and_then(s).unwrap_or("");
    match (ty, pty) {
        ("session_meta", _) => {
            acc.touch(ts);
            if let Some(c) = p.get("cwd").and_then(s) {
                acc.set_cwd(c)
            }
        }
        ("turn_context", _) => {
            if let Some(c) = p.get("cwd").and_then(s) {
                acc.set_cwd(c)
            }
            if let Some(m) = p.get("model").and_then(s) {
                acc.agent.model = Some(m.to_string());
            }
        }
        ("event_msg", "thread_settings_applied") => {
            if let Some(m) = p.pointer("/thread_settings/model").and_then(s) {
                acc.agent.model = Some(m.to_string());
                acc.dirty = true;
            }
        }
        ("event_msg", "token_count") => {
            if let Some(u) = p.pointer("/info/total_token_usage") {
                let n = |k: &str| u.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
                let cached = n("cached_input_tokens");
                let tk = Tokens {
                    input: n("input_tokens").saturating_sub(cached),
                    output: n("output_tokens"),
                    cache_read: cached,
                    cache_write: n("cache_write_input_tokens"),
                };
                if tk.total() >= acc.agent.tokens.total() {
                    acc.agent.tokens = tk;
                    acc.dirty = true;
                }
            }
        }
        ("event_msg", "thread_goal_updated") => {
            if let Some(o) = p.pointer("/goal/objective").and_then(s) {
                acc.set_title(o, TitleRank::Explicit)
            }
        }
        ("event_msg", "thread_name_updated") => {
            if let Some(n) = p.get("thread_name").or(p.get("name")).and_then(s) {
                acc.set_label(n, 1)
            }
        }
        ("event_msg", "task_started") => acc.touch(ts),
        ("event_msg", "user_message") => {
            let text = p.get("message").and_then(s).unwrap_or("").to_string();
            user_text(acc, &text, t, ts);
        }
        ("event_msg", "agent_message") => {
            acc.touch(ts);
            if let Some(m) = p.get("message").and_then(s) {
                acc.set_last_text(m)
            }
        }
        ("event_msg", "task_complete") => {
            acc.touch(ts);
            if let Some(m) = p.get("last_agent_message").and_then(s) {
                acc.set_last_text(m)
            }
        }
        ("event_msg", "item_completed") => {
            if p.pointer("/item/type").and_then(s) == Some("UserMessage") {
                let text = p.pointer("/item/content").map(content_text).unwrap_or_default();
                user_text(acc, &text, t, ts);
            }
        }
        ("response_item", "message") => {
            if p.get("role").and_then(s) == Some("user") {
                let text = p.get("content").map(content_text).unwrap_or_default();
                acc.scan_received(&text, t);
            } else if p.get("role").and_then(s) == Some("assistant") {
                acc.touch(ts);
                let text = p.get("content").map(content_text).unwrap_or_default();
                acc.set_last_text(&text);
            }
        }
        ("response_item", "custom_tool_call") => {
            acc.touch(ts);
            let name = p.get("name").and_then(s).unwrap_or("?");
            let input = p.get("input").and_then(s).unwrap_or("");
            let call = p.get("call_id").and_then(s).unwrap_or("");
            custom_call(acc, name, input, call, t);
        }
        ("response_item", "function_call") => {
            acc.touch(ts);
            let name = p.get("name").and_then(s).unwrap_or("?");
            let call = p.get("call_id").and_then(s).unwrap_or("");
            let args: Value = p
                .get("arguments")
                .and_then(s)
                .and_then(|a| serde_json::from_str(a).ok())
                .unwrap_or(Value::Null);
            function_call(acc, name, &args, call, t);
        }
        ("response_item", "custom_tool_call_output") | ("response_item", "function_call_output") => {
            let call = p.get("call_id").and_then(s).unwrap_or("");
            if let Some(pending) = acc.pending.remove(call) {
                let out = match p.get("output") {
                    Some(Value::String(x)) => x.clone(),
                    Some(o) => content_text(o),
                    None => String::new(),
                };
                match pending {
                    Pending::Send { stub_id } => acc.send_result(&stub_id, &out),
                    Pending::Subagent { id } => {
                        if let Some(sa) = acc.subagent_mut(&id) {
                            sa.last_ms = t;
                        }
                    }
                    Pending::File { .. } => {}
                }
            }
        }
        ("response_item", "agent_message") => {
            // Messages between this thread and its spawned agents.
            let author = p.get("author").and_then(s).unwrap_or("");
            let name = author.rsplit('/').next().unwrap_or("");
            if !name.is_empty() && name != "root" {
                if let Some(sa) = acc.subagent_mut(name) {
                    sa.last_ms = t;
                }
            }
        }
        _ => {}
    }
}

fn user_text(acc: &mut Acc, text: &str, t: i64, ts: Option<i64>) {
    acc.scan_received(text, t);
    if !text.contains("<agent-relay-message") && looks_like_prompt(text) || text.contains("## My request") {
        acc.touch(ts);
        acc.set_title(text, TitleRank::FirstPrompt);
    }
}

fn custom_call(acc: &mut Acc, name: &str, input: &str, call: &str, t: i64) {
    static CMD: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?s)\b(?:cmd|command)\s*:\s*(?:"((?:[^"\\]|\\.)*)"|'((?:[^'\\]|\\.)*)'|`((?:[^`\\]|\\.)*)`)"#).unwrap()
    });
    static TOOL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"tools\.([A-Za-z_][\w]*)\s*\(").unwrap());
    acc.tool(name);
    if name == "exec" {
        for c in TOOL.captures_iter(input) {
            acc.tool(&c[1]);
        }
        for c in CMD.captures_iter(input) {
            let raw = c.get(1).or(c.get(2)).or(c.get(3)).map(|m| m.as_str()).unwrap_or("");
            let cmd = unescape_js(raw);
            if let Some(stub) = acc.command(&cmd, t, Some(call)) {
                acc.pending.insert(call.to_string(), Pending::Send { stub_id: stub });
            }
        }
    }
    patch_files(acc, input, t);
}

fn function_call(acc: &mut Acc, name: &str, args: &Value, call: &str, t: i64) {
    acc.tool(name);
    let get = |k: &str| args.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    match name {
        "exec_command" | "shell" | "local_shell" => {
            let cmd = match args.get("cmd").or(args.get("command")) {
                Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(" "),
                Some(Value::String(x)) => x.clone(),
                _ => String::new(),
            };
            if let Some(stub) = acc.command(&cmd, t, Some(call)) {
                acc.pending.insert(call.to_string(), Pending::Send { stub_id: stub });
            }
        }
        "apply_patch" => patch_files(acc, &get("input"), t),
        "spawn_agent" => {
            let task_name = get("task_name");
            let msg = get("message");
            let task = if msg.starts_with("gAAAA") { "(task text is encrypted in the rollout)".into() } else { truncate(&msg, 600) };
            let id = if task_name.is_empty() { call.to_string() } else { task_name.clone() };
            acc.subagent(SubAgent {
                id: id.clone(),
                name: if task_name.is_empty() { "agent".into() } else { task_name },
                kind: "agent".into(),
                task,
                status: "running".into(),
                tokens: 0,
                cache_read: 0,
                started_ms: t,
                last_ms: t,
            });
            acc.pending.insert(call.to_string(), Pending::Subagent { id });
        }
        "followup_task" | "send_message" | "interrupt_agent" => {
            let target = get("target");
            if let Some(sa) = acc.subagent_mut(&target) {
                sa.last_ms = t;
                if name == "interrupt_agent" {
                    sa.status = "done".into();
                }
            }
        }
        "update_plan" => {
            if let Some(plan) = args.get("plan").and_then(|x| x.as_array()) {
                if let Some(cur) = plan.iter().find(|x| x.get("status").and_then(s) == Some("in_progress")) {
                    acc.set_focus(cur.get("step").and_then(s).unwrap_or(""));
                }
            }
        }
        _ => {}
    }
}

fn patch_files(acc: &mut Acc, text: &str, t: i64) {
    static PATCH: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\*\*\* (Add|Update|Delete) File: ([^\n\\]+)").unwrap());
    for c in PATCH.captures_iter(text) {
        let op = match &c[1] {
            "Add" => FileOp::Created,
            "Delete" => FileOp::Deleted,
            _ => FileOp::Edited,
        };
        acc.file(c[2].trim(), op, t);
    }
}

fn unescape_js(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}
