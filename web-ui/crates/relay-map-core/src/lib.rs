//! Types shared by the Relay Map collector, hub and WebAssembly front end.
//!
//! Everything that crosses the wire (collector stdout over ssh, and the hub's
//! WebSocket) is one of these types, serialised as JSON.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Caps that keep an agent record small enough to resend whole on every change.
pub const MAX_FILES: usize = 200;
pub const MAX_GIT: usize = 120;
pub const MAX_SUBAGENTS: usize = 80;
pub const MAX_BODY_PREVIEW: usize = 4000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum AgentKind {
    Claude,
    Codex,
    Antigravity,
    Opencode,
    Pi,
    /// The Relay Map operator (broadcasts) and anything unrecognised.
    Operator,
}

impl AgentKind {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "claude" => Self::Claude,
            "codex" => Self::Codex,
            "antigravity" => Self::Antigravity,
            "opencode" => Self::Opencode,
            "pi" => Self::Pi,
            "operator" => Self::Operator,
            _ => return None,
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Antigravity => "antigravity",
            Self::Opencode => "opencode",
            Self::Pi => "pi",
            Self::Operator => "operator",
        }
    }
    /// Whether agent-relay can deliver a message to this kind of agent.
    pub fn relay_target(self) -> bool {
        matches!(self, Self::Claude | Self::Codex | Self::Antigravity | Self::Opencode | Self::Pi)
    }
}

/// `kind:ref@host`, the same address format agent-relay uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Addr {
    pub kind: String,
    pub reference: String,
    pub host: Option<String>,
}

impl Addr {
    pub fn parse(text: &str) -> Option<Addr> {
        let text = text.trim().trim_matches(|c| c == '\'' || c == '"');
        let (kind, rest) = text.split_once(':')?;
        if AgentKind::parse(kind).is_none() || rest.is_empty() {
            return None;
        }
        let (reference, host) = match rest.rsplit_once('@') {
            Some((r, h)) if !h.is_empty() && !r.is_empty() => (r.to_string(), Some(h.to_string())),
            _ => (rest.to_string(), None),
        };
        Some(Addr { kind: kind.to_string(), reference, host })
    }
    pub fn with_default_host(mut self, host: &str) -> Addr {
        if self.host.as_deref().is_none_or(|h| h.is_empty() || h == "local" || h == "localhost") {
            self.host = Some(host.to_string());
        }
        self
    }
    pub fn render(&self) -> String {
        match &self.host {
            Some(h) => format!("{}:{}@{}", self.kind, self.reference, h),
            None => format!("{}:{}", self.kind, self.reference),
        }
    }
}

pub fn agent_addr(kind: AgentKind, id: &str, host: &str) -> String {
    format!("{}:{}@{}", kind.as_str(), id, host)
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl Tokens {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }
    /// Input and output tokens, leaving out cache reads and writes. A session
    /// that re-reads a large cached context shows a big total but small fresh use.
    pub fn fresh(&self) -> u64 {
        self.input + self.output
    }
    pub fn add(&mut self, o: &Tokens) {
        self.input += o.input;
        self.output += o.output;
        self.cache_read += o.cache_read;
        self.cache_write += o.cache_write;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileOp {
    Created,
    Written,
    Edited,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileTouch {
    pub path: String,
    pub op: FileOp,
    pub count: u32,
    pub at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GitAction {
    /// `commit`, `push`, `pr create`, ...
    pub verb: String,
    pub command: String,
    pub at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubAgent {
    pub id: String,
    pub name: String,
    /// `subagent`, `thread`, `task`, `general-purpose`, ...
    pub kind: String,
    pub task: String,
    /// `running` or `done`.
    pub status: String,
    /// All tokens, cache included.
    pub tokens: u64,
    /// The part of `tokens` that was cache reads.
    #[serde(default)]
    pub cache_read: u64,
    pub started_ms: i64,
    pub last_ms: i64,
}

/// Everything the map knows about one agent session. The collector sends the
/// whole record each time it changes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Agent {
    /// `kind:id@host`.
    pub addr: String,
    pub kind: Option<AgentKind>,
    pub id: String,
    pub host: String,
    /// Label set with `relay name`, Codex thread name, or session title.
    pub label: Option<String>,
    pub cwd: Option<String>,
    /// High-level task: session title, goal, or first prompt.
    pub title: Option<String>,
    /// What it is doing right now (checklist item, focus, plan step).
    pub focus: Option<String>,
    pub last_text: Option<String>,
    /// `busy` / `idle` from the relay registry when registered.
    pub state: Option<String>,
    pub alive: Option<bool>,
    pub registered: bool,
    pub started_ms: i64,
    pub last_active_ms: i64,
    pub tokens: Tokens,
    pub cost_usd: Option<f64>,
    pub model: Option<String>,
    pub files: Vec<FileTouch>,
    pub git: Vec<GitAction>,
    pub skills: BTreeMap<String, u32>,
    pub tools: BTreeMap<String, u32>,
    pub subagents: Vec<SubAgent>,
    pub transcript: Option<String>,
    /// The agent that spawned this one (an Antigravity swarm orchestrator).
    #[serde(default)]
    pub parent: Option<String>,
    /// Filled in by the hub from the pool rules.
    pub pool: Option<String>,
    /// Added by the hub for addresses seen only in messages.
    #[serde(default)]
    pub ghost: bool,
}

impl Agent {
    pub fn display_name(&self) -> String {
        if let Some(l) = self.label.as_ref().filter(|s| !s.is_empty()) {
            return l.clone();
        }
        if let Some(c) = self.cwd.as_ref() {
            if let Some(base) = c.trim_end_matches('/').rsplit('/').next().filter(|s| !s.is_empty()) {
                return base.to_string();
            }
        }
        let short: String = self.id.chars().take(8).collect();
        format!("{} {}", self.kind.map(|k| k.as_str()).unwrap_or("agent"), short)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MsgSource {
    /// An envelope in `~/.agent-relay/inbox` (authoritative).
    Inbox,
    /// An `<agent-relay-message>` block that reached a transcript.
    Received,
    /// A `relay send` command in the sender's transcript.
    Sent,
    /// Sent by Relay Map itself.
    Broadcast,
    /// An agent framework's own messaging between its agents (Antigravity swarms).
    Swarm,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelayMessage {
    pub id: String,
    /// Sender address as written (`kind:ref@host`).
    pub from: String,
    /// Recipient address as written, host filled in where known.
    pub to: String,
    pub subject: Option<String>,
    pub body: String,
    /// Size of the full body in bytes.
    pub bytes: u64,
    pub sent_ms: i64,
    pub source: MsgSource,
    /// A collector-local id this message replaces (a `send` stub).
    #[serde(default)]
    pub replaces: Option<String>,
    /// Filled by the hub: the agent addresses the endpoints resolve to.
    #[serde(default)]
    pub from_agent: Option<String>,
    #[serde(default)]
    pub to_agent: Option<String>,
}

/// One line on a collector's stdout.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum CollectorEvent {
    Hello { host: String, version: String, roots: Vec<String> },
    Agent(Box<Agent>),
    Message(RelayMessage),
    /// The initial backfill finished.
    Ready { agents: usize, messages: usize, millis: u64 },
    Note { text: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pool {
    pub name: String,
    pub color: String,
    pub description: String,
    /// Members may only talk inside the pool; cross-pool traffic is flagged.
    pub isolated: bool,
    /// Made by the hub from computer + project for agents no configured pool matches.
    #[serde(default)]
    pub auto: bool,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
}

/// Project name for a working folder: the repository a worktree belongs to,
/// else the folder name; a home folder is "home".
pub fn project_of(cwd: &str) -> String {
    let parts: Vec<&str> = cwd.split('/').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return "(no folder)".into();
    }
    // …/repo/.claude/worktrees/<name>  →  repo
    if let Some(i) = parts.iter().position(|p| *p == ".claude" || *p == ".worktrees") {
        if i > 0 && parts.get(i + 1).map_or(false, |n| *n == "worktrees") || parts[i] == ".worktrees" {
            return parts[i - 1].to_string();
        }
    }
    // ~/.codex/worktrees/<id>/<repo>/…  →  repo
    if let Some(i) = parts.iter().position(|p| *p == ".codex") {
        if parts.get(i + 1) == Some(&"worktrees") {
            if let Some(repo) = parts.get(i + 3) {
                return repo.to_string();
            }
        }
    }
    if (parts.len() == 2 && (parts[0] == "Users" || parts[0] == "home")) || parts == ["root"] {
        return "home".into();
    }
    if parts.len() >= 3 && parts[0] == "private" && parts[1] == "tmp" || parts[0] == "tmp" {
        return "tmp".into();
    }
    parts[parts.len() - 1].to_string()
}

/// The same per-computer colour the front end uses.
pub fn host_color(host: &str) -> &'static str {
    const PALETTE: [&str; 7] = ["#ffd166", "#06d6a0", "#ef476f", "#8ecae6", "#c77dff", "#f4a261", "#90be6d"];
    let mut h: u32 = 2166136261;
    for b in host.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    PALETTE[(h as usize) % PALETTE.len()]
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostStatus {
    pub name: String,
    /// `local`, `ssh`, ...
    pub via: String,
    /// `connecting`, `backfilling`, `live`, `error`.
    pub state: String,
    pub detail: String,
    pub agents: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BroadcastTarget {
    pub addr: String,
    pub ok: bool,
    pub detail: String,
}

/// Hub → browser.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ServerMsg {
    Snapshot {
        now_ms: i64,
        self_host: String,
        agents: Vec<Agent>,
        messages: Vec<RelayMessage>,
        pools: Vec<Pool>,
        hosts: Vec<HostStatus>,
    },
    Agent(Box<Agent>),
    Message(RelayMessage),
    Host(HostStatus),
    BroadcastPlan { request_id: String, targets: Vec<String>, skipped: Vec<String> },
    /// The full pool list changed (a new computer + project pool appeared).
    Pools { pools: Vec<Pool> },
    BroadcastResult { request_id: String, target: BroadcastTarget, done: bool },
    Error { text: String },
}

/// Who a broadcast goes to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Scope {
    /// Every live agent that agent-relay can reach.
    All,
    Pool { name: String },
    Agents { addrs: Vec<String> },
}

/// Browser → hub.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ClientMsg {
    Broadcast {
        request_id: String,
        scope: Scope,
        subject: String,
        body: String,
        /// Only list who would receive it.
        dry_run: bool,
        /// Include agents that look idle for a long time / not alive.
        include_stale: bool,
    },
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addr_roundtrip() {
        let a = Addr::parse("claude:abc-123@laptop").unwrap();
        assert_eq!(a.kind, "claude");
        assert_eq!(a.reference, "abc-123");
        assert_eq!(a.host.as_deref(), Some("laptop"));
        let b = Addr::parse("codex:my thread").unwrap().with_default_host("here");
        assert_eq!(b.render(), "codex:my thread@here");
        assert!(Addr::parse("bogus:x").is_none());
        assert!(Addr::parse("'antigravity:1234@pc'").is_some());
    }

    #[test]
    fn projects() {
        assert_eq!(project_of("/Users/a/git/web-app"), "web-app");
        assert_eq!(project_of("/Users/a/git/api-server/.claude/worktrees/agent-abc"), "api-server");
        assert_eq!(project_of("/Users/a/.codex/worktrees/0001/worker-service"), "worker-service");
        assert_eq!(project_of("/home/a"), "home");
        assert_eq!(project_of("/"), "(no folder)");
    }

    #[test]
    fn truncate_respects_utf8() {
        assert_eq!(truncate("héllo", 2), "h…");
        assert_eq!(truncate("abc", 5), "abc");
    }
}

/// One turn of a session log, flattened for reading.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptEntry {
    /// Milliseconds since the epoch, 0 when the line had no time.
    pub t: i64,
    /// `user`, `assistant`, `thinking`, `tool`, `result` or `system`.
    pub role: String,
    /// Tool name for `tool` and `result` entries.
    #[serde(default)]
    pub name: Option<String>,
    pub text: String,
}

/// A session log as the agent panel shows it (`GET /api/transcript`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TranscriptView {
    pub addr: String,
    pub host: String,
    pub path: Option<String>,
    /// `local`, `ssh` or `victorialogs`.
    pub source: String,
    /// Size of the whole log in bytes, when known.
    pub bytes: u64,
    /// Bytes read for this view (the tail of the log when it is large).
    pub shown_bytes: u64,
    /// Lines that did not parse or carried nothing readable.
    pub skipped: u64,
    pub entries: Vec<TranscriptEntry>,
    /// Why an earlier source was passed over, one line each.
    #[serde(default)]
    pub notes: Vec<String>,
}
