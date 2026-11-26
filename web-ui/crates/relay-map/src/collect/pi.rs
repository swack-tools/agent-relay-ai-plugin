//! pi coding agent sessions: `~/.pi/agent/sessions/<cwd>/<time>_<id>.jsonl`.

use super::common::*;
use relay_map_core::*;
use serde_json::Value;

pub fn line(acc: &mut Acc, v: &Value) {
    let ts = v.get("timestamp").and_then(parse_ts);
    let t = ts.unwrap_or(acc.agent.last_active_ms);
    match v.get("type").and_then(s).unwrap_or("") {
        "session" => {
            acc.touch(ts);
            if let Some(c) = v.get("cwd").and_then(s) {
                acc.set_cwd(c)
            }
        }
        "model_change" => {
            if let Some(m) = v.get("modelId").and_then(s) {
                acc.agent.model = Some(m.to_string());
                acc.dirty = true;
            }
        }
        "message" => {
            let Some(m) = v.get("message") else { return };
            match m.get("role").and_then(s) {
                Some("user") => {
                    let text = m.get("content").map(content_text).unwrap_or_default();
                    acc.touch(ts);
                    acc.scan_received(&text, t);
                    if looks_like_prompt(&text) {
                        acc.set_title(&text, TitleRank::FirstPrompt);
                    }
                }
                Some("assistant") => {
                    acc.touch(ts);
                    if let Some(u) = m.get("usage") {
                        let n = |k: &str| u.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
                        acc.tokens_add(&Tokens {
                            input: n("input"),
                            output: n("output"),
                            cache_read: n("cacheRead"),
                            cache_write: n("cacheWrite"),
                        });
                    }
                    if let Some(items) = m.get("content").and_then(|x| x.as_array()) {
                        for it in items {
                            match it.get("type").and_then(s) {
                                Some("text") => acc.set_last_text(it.get("text").and_then(s).unwrap_or("")),
                                Some("toolCall") => {
                                    let name = it.get("name").and_then(s).unwrap_or("?");
                                    let a = it.get("arguments").cloned().unwrap_or(Value::Null);
                                    let get = |k: &str| a.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
                                    acc.tool(name);
                                    match name {
                                        "bash" => {
                                            acc.command(&get("command"), t, it.get("id").and_then(s));
                                        }
                                        "write" => acc.file(&get("path"), FileOp::Written, t),
                                        "edit" => acc.file(&get("path"), FileOp::Edited, t),
                                        "read" => {
                                            for sk in skill_paths(&get("path")) {
                                                acc.skill(&sk)
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}
