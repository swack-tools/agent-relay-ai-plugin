//! Shared pieces of the transcript parsers: the per-agent accumulator and the
//! extractors for git commands, `relay send` commands and relayed messages.

use regex::Regex;
use relay_map_core::*;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

pub fn parse_ts(v: &Value) -> Option<i64> {
    match v {
        Value::String(s) => chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp_millis()),
        Value::Number(n) => n.as_i64().map(|x| if x < 10_000_000_000 { x * 1000 } else { x }),
        _ => None,
    }
}

pub fn s(v: &Value) -> Option<&str> {
    v.as_str()
}

/// Text of a content field that is either a string or a list of `{type, text}` parts.
pub fn content_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|i| i.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// What a pending tool call was, so its result can refine the record.
#[derive(Debug, Clone)]
pub enum Pending {
    File { path: String },
    Send { stub_id: String },
    Subagent { id: String },
}

/// Title candidates, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TitleRank {
    FirstPrompt = 1,
    Generated = 2,
    Explicit = 3,
}

/// Everything a collector learns about one agent while reading its files.
pub struct Acc {
    pub agent: Agent,
    pub title_rank: Option<TitleRank>,
    pub label_rank: u8,
    pub pending: HashMap<String, Pending>,
    pub seen_usage: HashSet<String>,
    pub seen_msgs: HashSet<String>,
    pub file_idx: HashMap<String, usize>,
    pub dirty: bool,
    /// Messages found while parsing, drained by the collector.
    pub out_msgs: Vec<RelayMessage>,
    stubs: HashMap<String, RelayMessage>,
    stub_seq: u32,
}

impl Acc {
    pub fn new(kind: AgentKind, id: &str, host: &str) -> Acc {
        Acc {
            agent: Agent {
                addr: agent_addr(kind, id, host),
                kind: Some(kind),
                id: id.to_string(),
                host: host.to_string(),
                ..Default::default()
            },
            title_rank: None,
            label_rank: 0,
            pending: HashMap::new(),
            seen_usage: HashSet::new(),
            seen_msgs: HashSet::new(),
            file_idx: HashMap::new(),
            dirty: true,
            out_msgs: Vec::new(),
            stubs: HashMap::new(),
            stub_seq: 0,
        }
    }

    pub fn touch(&mut self, ts: Option<i64>) {
        if let Some(t) = ts {
            if self.agent.started_ms == 0 || t < self.agent.started_ms {
                self.agent.started_ms = t;
            }
            if t > self.agent.last_active_ms {
                self.agent.last_active_ms = t;
            }
            self.dirty = true;
        }
    }

    pub fn set_title(&mut self, title: &str, rank: TitleRank) {
        let t = clean_title(title);
        if t.is_empty() {
            return;
        }
        if self.title_rank.is_none_or(|r| rank > r) {
            self.agent.title = Some(truncate(&t, 240));
            self.title_rank = Some(rank);
            self.dirty = true;
        }
    }

    /// rank: 1 = session name, 2 = relay label.
    pub fn set_label(&mut self, label: &str, rank: u8) {
        if label.trim().is_empty() || rank < self.label_rank {
            return;
        }
        self.agent.label = Some(truncate(label.trim(), 80));
        self.label_rank = rank;
        self.dirty = true;
    }

    pub fn set_cwd(&mut self, cwd: &str) {
        if !cwd.is_empty() && self.agent.cwd.as_deref() != Some(cwd) {
            self.agent.cwd = Some(cwd.to_string());
            self.dirty = true;
        }
    }

    pub fn set_focus(&mut self, focus: &str) {
        let f = focus.trim();
        if !f.is_empty() {
            self.agent.focus = Some(truncate(f, 200));
            self.dirty = true;
        }
    }

    pub fn set_last_text(&mut self, text: &str) {
        let t = text.trim();
        if !t.is_empty() {
            self.agent.last_text = Some(truncate(t, 600));
            self.dirty = true;
        }
    }

    pub fn tool(&mut self, name: &str) {
        *self.agent.tools.entry(name.to_string()).or_default() += 1;
        self.dirty = true;
    }

    pub fn skill(&mut self, name: &str) {
        let n = name.trim().trim_start_matches('/');
        if !n.is_empty() && n.len() < 80 {
            *self.agent.skills.entry(n.to_string()).or_default() += 1;
            self.dirty = true;
        }
    }

    pub fn file(&mut self, path: &str, op: FileOp, ts: i64) {
        let path = path.trim().trim_matches('"');
        if path.is_empty() {
            return;
        }
        if let Some(&i) = self.file_idx.get(path) {
            let f = &mut self.agent.files[i];
            f.count += 1;
            f.at_ms = f.at_ms.max(ts);
            // A file created in this session stays "created" however often it is edited.
            if f.op != FileOp::Created {
                f.op = op;
            }
        } else {
            if self.agent.files.len() >= MAX_FILES {
                // Drop the oldest entry.
                let (oldest, _) = self.agent.files.iter().enumerate().min_by_key(|(_, f)| f.at_ms).unwrap();
                self.agent.files.remove(oldest);
                self.file_idx = self.agent.files.iter().enumerate().map(|(i, f)| (f.path.clone(), i)).collect();
            }
            self.file_idx.insert(path.to_string(), self.agent.files.len());
            self.agent.files.push(FileTouch { path: path.to_string(), op, count: 1, at_ms: ts });
        }
        self.dirty = true;
    }

    pub fn file_result(&mut self, path: &str, created: bool) {
        if let Some(&i) = self.file_idx.get(path) {
            if created {
                self.agent.files[i].op = FileOp::Created;
                self.dirty = true;
            }
        }
    }

    pub fn tokens_add(&mut self, t: &Tokens) {
        self.agent.tokens.add(t);
        self.dirty = true;
    }

    pub fn subagent(&mut self, sub: SubAgent) {
        if let Some(existing) = self.agent.subagents.iter_mut().find(|x| x.id == sub.id) {
            existing.last_ms = existing.last_ms.max(sub.last_ms);
            if !sub.task.is_empty() && existing.task.is_empty() {
                existing.task = sub.task;
            }
            existing.tokens = existing.tokens.max(sub.tokens);
            existing.cache_read = existing.cache_read.max(sub.cache_read);
        } else {
            if self.agent.subagents.len() >= MAX_SUBAGENTS {
                self.agent.subagents.remove(0);
            }
            self.agent.subagents.push(sub);
        }
        self.dirty = true;
    }

    pub fn subagent_mut(&mut self, id: &str) -> Option<&mut SubAgent> {
        self.dirty = true;
        self.agent.subagents.iter_mut().find(|x| x.id == id)
    }

    /// Scan a shell command for git activity and `relay send` calls.
    /// Returns the stub id when the command sends a relay message.
    pub fn command(&mut self, cmd: &str, ts: i64, call_id: Option<&str>) -> Option<String> {
        for (verb, command) in git_actions(cmd) {
            if self.agent.git.len() >= MAX_GIT {
                self.agent.git.remove(0);
            }
            self.agent.git.push(GitAction { verb, command, at_ms: ts });
            self.dirty = true;
        }
        for skill in skill_paths(cmd) {
            self.skill(&skill);
        }
        let mut stub = None;
        for send in relay_sends(cmd) {
            self.stub_seq += 1;
            let id = format!("send:{}:{}:{}", self.agent.id, call_id.unwrap_or("x"), self.stub_seq);
            let to = Addr::parse(&send.to)
                .map(|a| a.with_default_host(&self.agent.host).render())
                .unwrap_or(send.to.clone());
            let msg = RelayMessage {
                id: id.clone(),
                from: self.agent.addr.clone(),
                to,
                subject: send.subject,
                bytes: send.bytes,
                body: truncate(&send.body, MAX_BODY_PREVIEW),
                sent_ms: ts,
                source: MsgSource::Sent,
                replaces: None,
                from_agent: None,
                to_agent: None,
            };
            if self.stubs.len() > 500 {
                self.stubs.clear();
            }
            self.stubs.insert(id.clone(), msg.clone());
            self.out_msgs.push(msg);
            stub = Some(id);
        }
        stub
    }

    /// A tool result that may carry the real id of a message a `relay send` produced.
    pub fn send_result(&mut self, stub_id: &str, output: &str) {
        static SENT: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"sent (\d{8}T\d{6}-[0-9a-f]{8}) to (\S+) via").unwrap());
        static JID: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r#""id": "(\d{8}T\d{6}-[0-9a-f]{8})""#).unwrap());
        static JTO: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r#""(session|thread|conversation)": "([A-Za-z0-9_.\-]{8,})""#).unwrap()
        });
        let (id, to_hint) = if let Some(c) = SENT.captures(output) {
            (c[1].to_string(), Some(c[2].to_string()))
        } else if let Some(c) = JID.captures(output) {
            (c[1].to_string(), None)
        } else {
            return;
        };
        let Some(stub) = self.stubs.remove(stub_id) else {
            return;
        };
        let mut to = stub.to.clone();
        if let Some(c) = JTO.captures(output) {
            if let Some(mut a) = Addr::parse(&to) {
                a.reference = c[2].to_string();
                to = a.render();
            }
        } else if let Some(h) = to_hint.and_then(|t| Addr::parse(&t)) {
            // `relay` prints the address it delivered to; keep the stub's host if it has none.
            let host = h.host.clone().or(Addr::parse(&to).and_then(|a| a.host));
            to = Addr { host, ..h }.render();
        }
        self.out_msgs.retain(|m| m.id != stub_id);
        self.out_msgs.push(RelayMessage { id, to, replaces: Some(stub_id.to_string()), ..stub });
    }

    /// Look for `<agent-relay-message>` blocks addressed to this agent.
    pub fn scan_received(&mut self, text: &str, ts: i64) {
        if !text.contains("<agent-relay-message") {
            return;
        }
        for m in received_messages(text) {
            if !self.seen_msgs.insert(m.id.clone()) {
                continue;
            }
            self.out_msgs.push(RelayMessage {
                id: m.id,
                from: m.from,
                to: self.agent.addr.clone(),
                subject: m.subject,
                bytes: m.body.len() as u64,
                body: truncate(&m.body, MAX_BODY_PREVIEW),
                sent_ms: m.sent_ms.unwrap_or(ts),
                source: MsgSource::Received,
                replaces: None,
                from_agent: None,
                to_agent: None,
            });
        }
    }
}

/// Strip wrappers from a prompt so it reads as a task.
pub fn clean_title(text: &str) -> String {
    static NOTE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<note>\s*(.*?)\s*</note>").unwrap());
    static TAGS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?s)<(system-reminder|ADDITIONAL_METADATA|local-command-stdout|command-message|command-args)>.*?</\w[\w-]*>").unwrap()
    });
    static ANY_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"</?[A-Za-z_][\w\-]*(\s[^>]*)?>").unwrap());
    static REQ: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)## My request(?: for Codex)?:\s*(.*)").unwrap());
    let mut t = text.to_string();
    if let Some(c) = NOTE.captures(&t) {
        t = c[1].to_string();
    }
    if let Some(c) = REQ.captures(&t) {
        t = c[1].to_string();
    }
    t = TAGS.replace_all(&t, " ").to_string();
    t = ANY_TAG.replace_all(&t, " ").to_string();
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    t.trim().to_string()
}

/// Whether a user-role text is a real prompt rather than tooling noise.
pub fn looks_like_prompt(text: &str) -> bool {
    let t = text.trim_start();
    if t.is_empty() {
        return false;
    }
    if t.starts_with("<relay ") || t.starts_with("<note>") || t.starts_with("<USER_REQUEST>") || t.contains("## My request") {
        return true;
    }
    !(t.starts_with('<') || t.starts_with("Caveat:") || t.starts_with("[Request interrupted") || t.starts_with("This session is being continued"))
}

pub fn git_actions(cmd: &str) -> Vec<(String, String)> {
    static GIT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?:^|[\s;&|(`])(git(?:\s+(?:-C\s+\S+|-c\s+\S+|--no-pager|--git-dir=\S+|--work-tree=\S+))*\s+([a-z][a-z\-]+)(?:\s+([a-z][a-z\-]+))?)").unwrap()
    });
    static GH: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?:^|[\s;&|(`])(gh\s+(pr|issue|run|repo|release|api|workflow|search)(?:\s+(?:-R\s+\S+\s+|--repo\s+\S+\s+)?([a-z][a-z\-]+))?)").unwrap()
    });
    let mut out = Vec::new();
    for seg in cmd.split(['\n', ';']).flat_map(|s| s.split("&&")).flat_map(|s| s.split("||")) {
        let seg = seg.trim();
        if seg.is_empty() || seg.starts_with('#') {
            continue;
        }
        if let Some(c) = GIT.captures(seg) {
            let verb = &c[2];
            // `git -C x worktree add` style: keep two words for compound verbs.
            let verb = match (verb, c.get(3).map(|m| m.as_str())) {
                ("worktree" | "stash" | "remote" | "submodule", Some(sub)) => format!("{verb} {sub}"),
                _ => verb.to_string(),
            };
            let start = c.get(1).unwrap().start();
            out.push((verb, truncate(seg[start..].trim(), 200)));
        } else if let Some(c) = GH.captures(seg) {
            let verb = match c.get(3) {
                Some(s) => format!("gh {} {}", &c[2], s.as_str()),
                None => format!("gh {}", &c[2]),
            };
            let start = c.get(1).unwrap().start();
            out.push((verb, truncate(seg[start..].trim(), 200)));
        }
    }
    out
}

pub fn skill_paths(text: &str) -> Vec<String> {
    static SK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"skills/([A-Za-z0-9_.\-]+)/SKILL\.md").unwrap());
    SK.captures_iter(text).map(|c| c[1].to_string()).collect()
}

pub struct SendCmd {
    pub to: String,
    pub subject: Option<String>,
    pub body: String,
    pub bytes: u64,
}

/// Find `relay send --to ADDR --message ...` invocations in a shell command.
pub fn relay_sends(cmd: &str) -> Vec<SendCmd> {
    static START: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"relay(?:\.py)?['"]?\s+send\b"#).unwrap());
    static TO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"--to[=\s]+['"]?([a-z]+:[^\s'"]+)"#).unwrap());
    if !cmd.contains("send") || !cmd.contains("relay") {
        return Vec::new();
    }
    let mut out = Vec::new();
    for m in START.find_iter(cmd) {
        // Only real invocations: the line must start the command (python3 …relay.py,
        // relay, env …), not quote it in a comment, a heredoc or a string.
        let line_start = cmd[..m.start()].rfind('\n').map_or(0, |i| i + 1);
        let head = cmd[line_start..m.start()].trim_start();
        let head = head.rsplit(|c| c == ';' || c == '&' || c == '|').next().unwrap_or(head).trim_start();
        let ok = head.is_empty()
            || head.starts_with("python")
            || head.starts_with("env ")
            || head.starts_with('/')
            || head.starts_with('~')
            || head.starts_with('"')
            || head.starts_with("ssh ");
        // Text after a heredoc start is file content, not a command.
        if !ok || head.contains('#') || head.contains('`') || cmd[..m.start()].contains("<<") {
            continue;
        }
        let rest = &cmd[m.end()..];
        let tokens = shell_words::split(rest).ok();
        let mut to = None;
        let mut subject = None;
        let mut body = String::new();
        let mut bytes = 0u64;
        if let Some(tokens) = tokens {
            let mut it = tokens.iter().peekable();
            while let Some(t) = it.next() {
                match t.as_str() {
                    "&&" | "||" | ";" | "|" => break,
                    "--to" => to = it.next().cloned(),
                    "--subject" => subject = it.next().cloned(),
                    "--message" => {
                        if let Some(b) = it.next() {
                            body = strip_heredoc(b);
                            bytes = body.len() as u64;
                        }
                    }
                    "--message-b64" => {
                        if let Some(b) = it.next() {
                            bytes = (b.len() as u64 * 3) / 4;
                            body = "(base64 body)".into();
                        }
                    }
                    "--file" => {
                        if let Some(p) = it.next() {
                            body = format!("(body in file {p})");
                        }
                    }
                    t if t.starts_with("--to=") => to = Some(t[5..].to_string()),
                    t if t.starts_with("--message=") => {
                        body = t[10..].to_string();
                        bytes = body.len() as u64;
                    }
                    _ => {}
                }
            }
        }
        if to.is_none() {
            to = TO.captures(rest).map(|c| c[1].to_string());
            if body.is_empty() {
                body = truncate(rest, 600);
                bytes = rest.len() as u64;
            }
        }
        if let Some(to) = to.filter(|t| !t.starts_with('$') && !t.contains('<') && Addr::parse(t).is_some()) {
            out.push(SendCmd { to, subject, body, bytes });
        }
    }
    out
}

fn strip_heredoc(b: &str) -> String {
    let t = b.trim();
    if let Some(rest) = t.strip_prefix("$(cat <<") {
        // $(cat <<'EOF'\n...\nEOF\n)
        let body = rest.split_once('\n').map(|x| x.1).unwrap_or("");
        let body = body.trim_end().trim_end_matches(')').trim_end();
        let body = body.rsplit_once('\n').map(|x| x.0).unwrap_or(body);
        return body.to_string();
    }
    t.to_string()
}

pub struct Received {
    pub id: String,
    pub from: String,
    pub subject: Option<String>,
    pub sent_ms: Option<i64>,
    pub body: String,
}

pub fn received_messages(text: &str) -> Vec<Received> {
    static RX: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?s)<agent-relay-message id="([^"]+)" from="([^"]*)" sent="([^"]*)"(?: subject="([^"]*)")?>\n?(.*?)</agent-relay-message>"#).unwrap()
    });
    RX.captures_iter(text)
        .map(|c| Received {
            id: c[1].to_string(),
            from: c[2].to_string(),
            sent_ms: chrono::DateTime::parse_from_rfc3339(&c[3]).ok().map(|d| d.timestamp_millis()),
            subject: c.get(4).map(|m| m.as_str().to_string()),
            body: c[5].trim().to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_git_and_gh() {
        let a = git_actions("cd x && git -C /r commit -m 'wip' && gh pr create --fill; git push -u origin b");
        let verbs: Vec<_> = a.iter().map(|x| x.0.as_str()).collect();
        assert_eq!(verbs, ["commit", "gh pr create", "push"]);
    }

    #[test]
    fn finds_relay_send() {
        let s = relay_sends("python3 /p/scripts/relay.py send --to claude:abc@laptop --subject hi --message 'hello there' && echo ok");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].to, "claude:abc@laptop");
        assert_eq!(s[0].body, "hello there");
        assert_eq!(s[0].subject.as_deref(), Some("hi"));
        assert!(relay_sends("relay.py send --to $s --message x").is_empty());
        assert!(relay_sends("cat > t.rs <<'EOF'\n// A `relay send --to claude:x@far` stub\nEOF").is_empty());
        assert_eq!(relay_sends("cd /x && python3 /p/relay.py send --to codex:t --message hi").len(), 1);
        assert!(relay_sends("python3 - <<'EOF'\nx = 'python3 /p/relay.py send --to codex:t'\nEOF").is_empty());
    }

    #[test]
    fn parses_received_block() {
        let t = "x <agent-relay-message id=\"20260101T120000-0a1b2c3d\" from=\"claude:e6@laptop\" sent=\"2026-01-01T12:00:00-06:00\">\nCheck-in\n</agent-relay-message> y";
        let r = received_messages(t);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].from, "claude:e6@laptop");
        assert_eq!(r[0].body, "Check-in");
        assert!(r[0].sent_ms.is_some());
    }

    #[test]
    fn title_from_relay_note() {
        let t = clean_title("<relay from=\"coordinator\"><note>  Build the map </note></relay>");
        assert_eq!(t, "Build the map");
    }
}
