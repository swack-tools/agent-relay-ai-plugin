//! Claude Code transcripts: `~/.claude/projects/<project>/<session>.jsonl` and
//! subagent transcripts in `<session>/subagents/agent-<id>.jsonl`.

use super::common::*;
use relay_map_core::*;
use serde_json::Value;

pub fn line(acc: &mut Acc, v: &Value) {
    let ts = v.get("timestamp").and_then(parse_ts);
    let t = ts.unwrap_or(acc.agent.last_active_ms);
    let ty = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
    if let Some(cwd) = v.get("cwd").and_then(|x| x.as_str()) {
        acc.set_cwd(cwd);
    }
    match ty {
        "custom-title" => {
            if let Some(x) = v.get("customTitle").and_then(s) {
                acc.set_title(x, TitleRank::Explicit)
            }
        }
        "ai-title" => {
            if let Some(x) = v.get("aiTitle").and_then(s) {
                acc.set_title(x, TitleRank::Generated)
            }
        }
        "summary" => {
            if let Some(x) = v.get("summary").and_then(s) {
                acc.set_title(x, TitleRank::Generated)
            }
        }
        "agent-name" => {
            if let Some(x) = v.get("agentName").and_then(s) {
                acc.set_label(x, 1)
            }
        }
        "relocated" => {
            if let Some(x) = v.get("relocatedCwd").and_then(s) {
                acc.set_cwd(x)
            }
        }
        "pr-link" => {
            let url = v.get("prUrl").and_then(s).unwrap_or("");
            acc.agent.git.push(GitAction { verb: "pr link".into(), command: url.to_string(), at_ms: t });
            acc.dirty = true;
        }
        "cost-state" => {
            if let Some(c) = v.get("totalCostUSD").and_then(|x| x.as_f64()) {
                acc.agent.cost_usd = Some(c);
                acc.dirty = true;
            }
        }
        "queue-operation" => {
            if let Some(c) = v.get("content").and_then(s) {
                acc.scan_received(c, t);
            }
        }
        "attachment" | "system" => {
            // Hook output (relayed messages delivered at SessionStart / prompt time).
            let raw = v.to_string();
            if raw.contains("agent-relay-message") {
                walk_strings(v, &mut |x| acc.scan_received(x, t));
            }
        }
        "user" => user(acc, v, t, ts),
        "assistant" => assistant(acc, v, t, ts),
        _ => {}
    }
}

fn walk_strings(v: &Value, f: &mut dyn FnMut(&str)) {
    match v {
        Value::String(x) => f(x),
        Value::Array(a) => a.iter().for_each(|i| walk_strings(i, f)),
        Value::Object(o) => o.values().for_each(|i| walk_strings(i, f)),
        _ => {}
    }
}

fn user(acc: &mut Acc, v: &Value, t: i64, ts: Option<i64>) {
    let Some(content) = v.pointer("/message/content") else { return };
    let meta = v.get("isMeta").and_then(|x| x.as_bool()).unwrap_or(false);
    let mut texts = Vec::new();
    match content {
        Value::String(x) => texts.push(x.clone()),
        Value::Array(items) => {
            for it in items {
                match it.get("type").and_then(s) {
                    Some("text") => {
                        if let Some(x) = it.get("text").and_then(s) {
                            texts.push(x.to_string())
                        }
                    }
                    Some("tool_result") => {
                        let id = it.get("tool_use_id").and_then(s).unwrap_or("");
                        let out = it.get("content").map(content_text).unwrap_or_default();
                        tool_result(acc, id, &out);
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    for x in &texts {
        acc.scan_received(x, t);
        if let Some(cmd) = x.split("<command-name>").nth(1).and_then(|r| r.split("</command-name>").next()) {
            acc.skill(cmd);
        }
        if !meta && looks_like_prompt(x) && !x.contains("<agent-relay-message") {
            acc.touch(ts);
            acc.set_title(x, TitleRank::FirstPrompt);
        }
    }
}

pub fn tool_result(acc: &mut Acc, id: &str, out: &str) {
    match acc.pending.remove(id) {
        Some(Pending::File { path }) => {
            let created = out.contains("created successfully") || out.contains("File created");
            acc.file_result(&path, created);
        }
        Some(Pending::Send { stub_id }) => acc.send_result(&stub_id, out),
        Some(Pending::Subagent { id }) => {
            if let Some(sa) = acc.subagent_mut(&id) {
                sa.status = "done".into();
            }
        }
        None => {}
    }
}

fn assistant(acc: &mut Acc, v: &Value, t: i64, ts: Option<i64>) {
    acc.touch(ts);
    let Some(msg) = v.get("message") else { return };
    if let Some(m) = msg.get("model").and_then(s) {
        if !m.starts_with('<') {
            acc.agent.model = Some(m.to_string());
        }
    }
    if let (Some(id), Some(u)) = (msg.get("id").and_then(s), msg.get("usage")) {
        if acc.seen_usage.insert(id.to_string()) {
            acc.tokens_add(&usage(u));
        }
    }
    let Some(items) = msg.get("content").and_then(|c| c.as_array()) else { return };
    for it in items {
        match it.get("type").and_then(s) {
            Some("text") => {
                if let Some(x) = it.get("text").and_then(s) {
                    acc.set_last_text(x)
                }
            }
            Some("tool_use") => {
                let id = it.get("id").and_then(s).unwrap_or("").to_string();
                let name = it.get("name").and_then(s).unwrap_or("?").to_string();
                let input = it.get("input").cloned().unwrap_or(Value::Null);
                tool_use(acc, &id, &name, &input, t);
            }
            _ => {}
        }
    }
}

pub fn usage(u: &Value) -> Tokens {
    let n = |k: &str| u.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    Tokens {
        input: n("input_tokens"),
        output: n("output_tokens"),
        cache_read: n("cache_read_input_tokens"),
        cache_write: n("cache_creation_input_tokens"),
    }
}

pub fn tool_use(acc: &mut Acc, id: &str, name: &str, input: &Value, t: i64) {
    acc.tool(name);
    let str_in = |k: &str| input.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    match name {
        "Bash" => {
            if let Some(stub) = acc.command(&str_in("command"), t, Some(id)) {
                acc.pending.insert(id.to_string(), Pending::Send { stub_id: stub });
            }
        }
        "Write" => {
            let p = str_in("file_path");
            acc.file(&p, FileOp::Written, t);
            acc.pending.insert(id.to_string(), Pending::File { path: p });
        }
        "Edit" | "MultiEdit" => acc.file(&str_in("file_path"), FileOp::Edited, t),
        "NotebookEdit" => acc.file(&str_in("notebook_path"), FileOp::Edited, t),
        "Read" => {
            for sk in skill_paths(&str_in("file_path")) {
                acc.skill(&sk)
            }
        }
        "Skill" => {
            let sk = if !str_in("skill").is_empty() { str_in("skill") } else { str_in("command") };
            acc.skill(&sk);
        }
        "Agent" | "Task" => {
            let desc = str_in("description");
            let prompt = str_in("prompt");
            acc.subagent(SubAgent {
                id: id.to_string(),
                name: if desc.is_empty() { truncate(&prompt, 60) } else { desc },
                kind: input.get("subagent_type").and_then(|x| x.as_str()).unwrap_or("subagent").to_string(),
                task: truncate(&prompt, 600),
                status: "running".into(),
                tokens: 0,
                cache_read: 0,
                started_ms: t,
                last_ms: t,
            });
            acc.pending.insert(id.to_string(), Pending::Subagent { id: id.to_string() });
        }
        "TodoWrite" => {
            if let Some(todos) = input.get("todos").and_then(|x| x.as_array()) {
                if let Some(cur) = todos.iter().find(|x| x.get("status").and_then(s) == Some("in_progress")) {
                    let f = cur.get("activeForm").or(cur.get("content")).and_then(s).unwrap_or("");
                    acc.set_focus(f);
                }
            }
        }
        n if n.ends_with("update_progress_checklist") => {
            let text = str_in("text");
            if let Some(line) = text.lines().find(|l| l.trim_start().starts_with('✱')) {
                acc.set_focus(line.trim_start().trim_start_matches('✱').trim());
            }
            if let Some(head) = text.lines().next() {
                acc.set_title(head, TitleRank::Generated);
            }
        }
        n if n.ends_with("rename_thread") => acc.set_title(&str_in("title"), TitleRank::Explicit),
        "mcp__ccd_session__start_session" | "mcp__ccd_session__hand_off_to_session" => {
            let title = str_in("title");
            acc.subagent(SubAgent {
                id: id.to_string(),
                name: truncate(&title, 60),
                kind: "session".into(),
                task: truncate(&str_in("prompt"), 600),
                status: "running".into(),
                tokens: 0,
                cache_read: 0,
                started_ms: t,
                last_ms: t,
            });
        }
        _ => {}
    }
}

/// A line from a subagent transcript. Work done there counts towards the parent.
pub fn subagent_line(acc: &mut Acc, sub_id: &str, v: &Value) {
    let ts = v.get("timestamp").and_then(parse_ts);
    let t = ts.unwrap_or(acc.agent.last_active_ms);
    let ty = v.get("type").and_then(s).unwrap_or("");
    let exists = acc.agent.subagents.iter().any(|x| x.id == sub_id);
    if !exists {
        let mut task = String::new();
        if ty == "user" {
            task = v.pointer("/message/content").map(content_text).unwrap_or_default();
        }
        // Merge with an Agent tool call that carried the same prompt.
        let key: String = task.chars().take(120).collect();
        if !key.is_empty() {
            if let Some(sa) = acc.agent.subagents.iter_mut().find(|x| x.task.starts_with(&key) || key.starts_with(&x.task.chars().take(120).collect::<String>()) && !x.task.is_empty()) {
                sa.id = sub_id.to_string();
            }
        }
        if !acc.agent.subagents.iter().any(|x| x.id == sub_id) {
            acc.subagent(SubAgent {
                id: sub_id.to_string(),
                name: truncate(clean_title(&task).as_str(), 60),
                kind: "subagent".into(),
                task: truncate(&task, 600),
                status: "running".into(),
                tokens: 0,
                cache_read: 0,
                started_ms: t,
                last_ms: t,
            });
        }
    }
    if ty == "assistant" {
        let (mut add, mut add_cr) = (0, 0);
        if let (Some(id), Some(u)) = (v.pointer("/message/id").and_then(s), v.pointer("/message/usage")) {
            if acc.seen_usage.insert(id.to_string()) {
                let tk = usage(u);
                add = tk.total();
                add_cr = tk.cache_read;
                acc.tokens_add(&tk);
            }
        }
        if let Some(items) = v.pointer("/message/content").and_then(|c| c.as_array()) {
            for it in items {
                if it.get("type").and_then(s) == Some("tool_use") {
                    let name = it.get("name").and_then(s).unwrap_or("?");
                    let input = it.get("input").cloned().unwrap_or(Value::Null);
                    let id = it.get("id").and_then(s).unwrap_or("");
                    if matches!(name, "Bash" | "Write" | "Edit" | "MultiEdit" | "NotebookEdit" | "Skill" | "Read") {
                        tool_use(acc, id, name, &input, t);
                    }
                }
            }
        }
        if let Some(sa) = acc.subagent_mut(sub_id) {
            sa.tokens += add;
            sa.cache_read += add_cr;
            sa.last_ms = sa.last_ms.max(t);
        }
    } else if ty == "user" {
        if let Some(items) = v.pointer("/message/content").and_then(|c| c.as_array()) {
            for it in items {
                if it.get("type").and_then(s) == Some("tool_result") {
                    let id = it.get("tool_use_id").and_then(s).unwrap_or("");
                    let out = it.get("content").map(content_text).unwrap_or_default();
                    tool_result(acc, id, &out);
                }
            }
        }
        if let Some(sa) = acc.subagent_mut(sub_id) {
            sa.last_ms = sa.last_ms.max(t);
        }
    }
}
