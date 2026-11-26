//! Antigravity transcripts:
//! `~/.gemini/antigravity{,-cli,-ide}/brain/<conversation>/.system_generated/logs/transcript.jsonl`.

use super::common::*;
use regex::Regex;
use relay_map_core::*;
use serde_json::Value;
use std::sync::LazyLock;

/// Tool arguments are JSON-encoded strings (`"Path": "\"/x\""`); decode them.
fn arg(args: &Value, keys: &[&str]) -> String {
    for k in keys {
        if let Some(v) = args.get(*k) {
            return match v {
                Value::String(x) => match serde_json::from_str::<Value>(x) {
                    Ok(Value::String(inner)) => inner,
                    _ => x.clone(),
                },
                other => other.to_string(),
            };
        }
    }
    String::new()
}

pub fn line(acc: &mut Acc, v: &Value) {
    let ts = v.get("created_at").and_then(parse_ts);
    let t = ts.unwrap_or(acc.agent.last_active_ms);
    let ty = v.get("type").and_then(s).unwrap_or("");
    let content = v.get("content").and_then(s).unwrap_or("");
    match ty {
        "USER_INPUT" => {
            acc.touch(ts);
            acc.scan_received(content, t);
            if !content.contains("<agent-relay-message") {
                let req = content
                    .split("<USER_REQUEST>")
                    .nth(1)
                    .and_then(|r| r.split("</USER_REQUEST>").next())
                    .unwrap_or(content);
                acc.set_title(req, TitleRank::FirstPrompt);
            }
        }
        "SYSTEM_MESSAGE" => {
            acc.scan_received(content, t);
            subagent_note(acc, content, t);
            let step = v.get("step_index").and_then(|x| x.as_u64()).unwrap_or(0);
            swarm_message(acc, content, step, t);
        }
        "PLANNER_RESPONSE" => {
            acc.touch(ts);
            let n = |k: &str| v.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
            acc.tokens_add(&Tokens {
                input: n("input_tokens"),
                output: n("output_tokens"),
                cache_read: n("cache_read_tokens"),
                cache_write: 0,
            });
            if !content.is_empty() {
                acc.set_last_text(content);
            }
            if let Some(calls) = v.get("tool_calls").and_then(|x| x.as_array()) {
                let step = v.get("step_index").and_then(|x| x.as_u64()).unwrap_or(0);
                for (i, c) in calls.iter().enumerate() {
                    let name = c.get("name").and_then(s).unwrap_or("?");
                    let args = c.get("args").cloned().unwrap_or(Value::Null);
                    tool(acc, name, &args, &format!("{step}.{i}"), t);
                }
            }
        }
        _ => {}
    }
}

fn tool(acc: &mut Acc, name: &str, args: &Value, call: &str, t: i64) {
    acc.tool(name);
    match name {
        "run_command" => {
            acc.command(&arg(args, &["CommandLine", "command"]), t, Some(call));
        }
        "write_file" | "write_to_file" => acc.file(&arg(args, &["Path", "TargetFile", "path"]), FileOp::Written, t),
        "edit_file" | "replace_file_content" | "multi_replace_file_content" => {
            acc.file(&arg(args, &["Path", "TargetFile", "path"]), FileOp::Edited, t)
        }
        "read_file" | "view_file" => {
            for sk in skill_paths(&arg(args, &["Path", "AbsolutePath", "path"])) {
                acc.skill(&sk)
            }
        }
        "invoke_subagent" => {
            static NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"You are `([^`]+)`").unwrap());
            let prompt = arg(args, &["Prompt", "prompt"]);
            let name = arg(args, &["Name", "TaskName", "name"]);
            let name = if !name.is_empty() {
                name
            } else {
                NAME.captures(&prompt).map(|c| c[1].to_string()).unwrap_or_else(|| truncate(&clean_title(&prompt), 50))
            };
            acc.subagent(SubAgent {
                id: name.clone(),
                name,
                kind: "subagent".into(),
                task: truncate(&prompt, 600),
                status: "running".into(),
                tokens: 0,
                cache_read: 0,
                started_ms: t,
                last_ms: t,
            });
        }
        "update_focus" => acc.set_focus(&arg(args, &["focus"])),
        "update_steps" => {
            if let Ok(Value::Array(steps)) = serde_json::from_str::<Value>(&arg(args, &["steps"])) {
                if let Some(cur) = steps.iter().find(|x| x.get("status").and_then(s) == Some("in_progress")) {
                    acc.set_focus(cur.get("title").and_then(s).unwrap_or(""));
                }
            }
        }
        "swarm" => {
            if arg(args, &["action"]) == "title" {
                acc.set_title(&arg(args, &["title"]), TitleRank::Generated);
            }
        }
        "skill" | "use_skill" => acc.skill(&arg(args, &["name", "Skill", "skill"])),
        _ => {}
    }
}

/// `[Message] ... sender=<conv>/task-12 ... finished with result:` marks a background task.
fn subagent_note(acc: &mut Acc, content: &str, t: i64) {
    static SENDER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"sender=[0-9a-f\-]{36}/(task-\d+)").unwrap());
    if let Some(c) = SENDER.captures(content) {
        let id = c[1].to_string();
        let done = content.contains("finished with result");
        acc.subagent(SubAgent {
            id: id.clone(),
            name: id.clone(),
            kind: "task".into(),
            task: truncate(content.split("content=").nth(1).unwrap_or(""), 300),
            status: if done { "done".into() } else { "running".into() },
            tokens: 0,
            cache_read: 0,
            started_ms: t,
            last_ms: t,
        });
        if done {
            if let Some(sa) = acc.subagent_mut(&id) {
                sa.status = "done".into();
            }
        }
    }
}

/// `[Message] timestamp=... sender=<conversation> ... content=...`: a message from
/// another conversation in the same Antigravity swarm. The first one a
/// conversation receives is the task its orchestrator spawned it with.
fn swarm_message(acc: &mut Acc, content: &str, step: u64, t: i64) {
    static MSG: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?s)\[Message\] timestamp=(\S+) sender=([0-9a-f\-]{36}) priority=\S+ content=(.*?)(?:</SYSTEM_MESSAGE>|$)").unwrap()
    });
    static NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[^\n]{0,200}?`([A-Za-z0-9_.\-]{3,60})`").unwrap());
    let Some(c) = MSG.captures(content) else { return };
    let sender = c[2].to_string();
    if sender == acc.agent.id {
        return;
    }
    let body = c[3].trim().to_string();
    let sent = chrono::DateTime::parse_from_rfc3339(&c[1]).ok().map(|d| d.timestamp_millis()).unwrap_or(t);
    let from = agent_addr(AgentKind::Antigravity, &sender, &acc.agent.host);
    if step <= 3 && acc.agent.parent.is_none() && acc.title_rank.is_none_or(|r| r < TitleRank::Explicit) {
        acc.agent.parent = Some(from.clone());
        if let Some(n) = NAME.captures(&body) {
            acc.set_label(&n[1], 1);
        }
        acc.set_title(&body, TitleRank::Generated);
    }
    let id = format!("swarm:{}:{}", acc.agent.id, step);
    if acc.seen_msgs.insert(id.clone()) {
        acc.out_msgs.push(RelayMessage {
            id,
            from,
            to: acc.agent.addr.clone(),
            subject: None,
            bytes: body.len() as u64,
            body: truncate(&body, MAX_BODY_PREVIEW),
            sent_ms: sent,
            source: MsgSource::Swarm,
            replaces: None,
            from_agent: None,
            to_agent: None,
        });
    }
}
