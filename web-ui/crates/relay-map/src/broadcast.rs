//! Broadcast a message to many agents through agent-relay's own `relay send`.

use crate::collect::now_ms;
use crate::hub::{operator_addr, Shared};
use relay_map_core::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;
use tokio::sync::Semaphore;

/// An agent counts as live for "everyone" when its process is alive or it was
/// active within this window.
const LIVE_WINDOW_MS: i64 = 2 * 3600 * 1000;

/// Find `relay.py`: config, env, this repository, then the installed plugin.
pub fn find_relay_py(configured: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = configured {
        return Some(p.to_path_buf());
    }
    if let Some(p) = std::env::var_os("AGENT_RELAY_PY") {
        return Some(PathBuf::from(p));
    }
    // The plugin the agents actually run, then a checkout of this repository.
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        let cache = home.join(".claude/plugins/cache/agent-relay/agent-relay");
        if let Ok(rd) = std::fs::read_dir(&cache) {
            let mut versions: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
            versions.sort();
            if let Some(p) = versions.into_iter().rev().map(|v| v.join("scripts/relay.py")).find(|p| p.is_file()) {
                return Some(p);
            }
        }
    }
    let rel = Path::new("plugins/agent-relay/scripts/relay.py");
    let mut starts = vec![std::env::current_dir().ok()];
    starts.push(std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)));
    for start in starts.into_iter().flatten() {
        let mut d = Some(start.as_path());
        while let Some(dir) = d {
            if dir.join(rel).is_file() {
                return Some(dir.join(rel));
            }
            d = dir.parent();
        }
    }
    None
}

pub fn plan(hub: &Shared, scope: &Scope, include_stale: bool) -> (Vec<String>, Vec<String>) {
    let h = hub.lock().unwrap();
    let now = now_ms();
    let mut targets = Vec::new();
    let mut skipped = Vec::new();
    let live = |a: &Agent| a.alive == Some(true) || now - a.last_active_ms < LIVE_WINDOW_MS;
    let consider: Vec<&Agent> = match scope {
        Scope::All => h.agents.values().collect(),
        Scope::Pool { name } => h.agents.values().filter(|a| a.pool.as_deref() == Some(name)).collect(),
        Scope::Agents { addrs } => addrs.iter().filter_map(|x| h.agents.get(x)).collect(),
    };
    let explicit = matches!(scope, Scope::Agents { .. });
    for a in consider {
        let kind = a.kind.unwrap_or(AgentKind::Operator);
        if kind == AgentKind::Operator {
            continue;
        }
        if a.ghost {
            skipped.push(format!("{} (seen only in messages)", a.addr));
        } else if !kind.relay_target() {
            skipped.push(format!("{} ({} is not an agent-relay target)", a.addr, kind.as_str()));
        } else if !explicit && !include_stale && !live(a) {
            skipped.push(format!("{} (idle more than 2 h)", a.addr));
        } else {
            targets.push(a.addr.clone());
        }
    }
    targets.sort();
    skipped.sort();
    (targets, skipped)
}

pub async fn send_all(hub: Shared, relay_py: Option<PathBuf>, request_id: String, scope: Scope, subject: String, body: String, targets: Vec<String>) {
    let Some(relay) = relay_py else {
        let _ = hub.lock().unwrap().tx.send(ServerMsg::Error { text: "relay.py not found; set relay_py in relay-map.toml".into() });
        return;
    };
    let (self_host, scope_text) = {
        let h = hub.lock().unwrap();
        let scope_text = match &scope {
            Scope::All => "every live agent".to_string(),
            Scope::Pool { name } => format!("every agent in the {name} pool"),
            Scope::Agents { addrs } => format!("{} selected agent(s)", addrs.len()),
        };
        (h.self_host.clone(), scope_text)
    };
    let from = operator_addr(&self_host);
    let full = format!(
        "{body}\n\n(Broadcast from the user's Relay Map dashboard to {scope_text}. It has no reply address; report back through your normal channel.)"
    );
    let sem = Arc::new(Semaphore::new(4));
    let total = targets.len();
    let done = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut tasks = Vec::new();
    for to in targets {
        let (sem, hub, relay, from, full, subject, rid, done) =
            (sem.clone(), hub.clone(), relay.clone(), from.clone(), full.clone(), subject.clone(), request_id.clone(), done.clone());
        tasks.push(tokio::spawn(async move {
            let _permit = sem.acquire().await;
            let mut cmd = Command::new("python3");
            cmd.arg(&relay).args(["send", "--to", &to, "--from", &from, "--message", &full, "--json"]);
            if !subject.is_empty() {
                cmd.args(["--subject", &subject]);
            }
            cmd.kill_on_drop(true);
            let sent_ms = now_ms();
            let res = tokio::time::timeout(Duration::from_secs(150), cmd.output()).await;
            let (ok, detail, id) = match res {
                Ok(Ok(out)) if out.status.success() => {
                    let text = String::from_utf8_lossy(&out.stdout).to_string();
                    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                    let id = v.get("id").and_then(|x| x.as_str()).map(str::to_string);
                    let d = v.get("delivery").and_then(|x| x.as_str()).unwrap_or("sent").to_string();
                    (true, d, id)
                }
                Ok(Ok(out)) => {
                    let e = String::from_utf8_lossy(&out.stderr).trim().to_string();
                    (false, if e.is_empty() { format!("exit {}", out.status) } else { e }, None)
                }
                Ok(Err(e)) => (false, format!("could not run python3: {e}"), None),
                Err(_) => (false, "timed out after 150 s".into(), None),
            };
            let n = done.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            let mut h = hub.lock().unwrap();
            if ok {
                h.add_message(RelayMessage {
                    id: id.unwrap_or_else(|| format!("broadcast:{rid}:{to}")),
                    from: from.clone(),
                    to: to.clone(),
                    subject: (!subject.is_empty()).then(|| subject.clone()),
                    bytes: full.len() as u64,
                    body: truncate(&full, MAX_BODY_PREVIEW),
                    sent_ms,
                    source: MsgSource::Broadcast,
                    replaces: None,
                    from_agent: None,
                    to_agent: None,
                });
            }
            let _ = h.tx.send(ServerMsg::BroadcastResult {
                request_id: rid,
                target: BroadcastTarget { addr: to, ok, detail },
                done: n == total,
            });
        }));
    }
    for t in tasks {
        let _ = t.await;
    }
}
