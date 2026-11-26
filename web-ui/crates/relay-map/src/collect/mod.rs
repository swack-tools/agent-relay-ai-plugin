//! The collector: tails the agent-relay registry and mailboxes plus every agent
//! transcript on one computer, and turns them into [`CollectorEvent`]s.
//!
//! It runs inside the hub for the local computer, and as `relay-map collect`
//! over ssh on remote ones (one JSON event per stdout line).

pub mod agy;
pub mod claude;
pub mod codex;
pub mod common;
pub mod pi;

use common::Acc;
use relay_map_core::*;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Src {
    Claude { sid: String },
    ClaudeSub { sid: String, aid: String },
    Codex { tid: String },
    Agy { cid: String },
    Pi { id: String },
}

impl Src {
    fn key(&self) -> (AgentKind, String) {
        match self {
            Src::Claude { sid } | Src::ClaudeSub { sid, .. } => (AgentKind::Claude, sid.clone()),
            Src::Codex { tid } => (AgentKind::Codex, tid.clone()),
            Src::Agy { cid } => (AgentKind::Antigravity, cid.clone()),
            Src::Pi { id } => (AgentKind::Pi, id.clone()),
        }
    }
}

struct Tracked {
    src: Src,
    offset: u64,
}

pub struct Collector {
    pub host: String,
    home: PathBuf,
    lookback_ms: i64,
    files: HashMap<PathBuf, Tracked>,
    accs: HashMap<(AgentKind, String), Acc>,
    inbox_seen: HashSet<PathBuf>,
    registry_seen: HashMap<PathBuf, SystemTime>,
    last_discover: Option<Instant>,
    msg_count: usize,
}

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

fn relay_home() -> PathBuf {
    std::env::var_os("AGENT_RELAY_HOME").map(PathBuf::from).unwrap_or_else(|| home_dir().join(".agent-relay"))
}

/// The name other computers use for this one, as agent-relay computes it.
pub fn self_host() -> String {
    if let Ok(h) = std::env::var("AGENT_RELAY_SELF") {
        if !h.is_empty() {
            return h;
        }
    }
    if let Ok(text) = fs::read_to_string(relay_home().join("config.json")) {
        if let Ok(v) = serde_json::from_str::<Value>(&text) {
            if let Some(h) = v.get("self").and_then(|x| x.as_str()) {
                return h.to_string();
            }
        }
    }
    let mut buf = [0u8; 256];
    // SAFETY: buf is valid for its length; gethostname NUL-terminates on success.
    let ok = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) } == 0;
    if ok {
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        let name = String::from_utf8_lossy(&buf[..end]).to_string();
        return name.split('.').next().unwrap_or(&name).to_string();
    }
    "localhost".into()
}

pub fn pid_alive(pid: i64) -> bool {
    if pid <= 1 {
        return false;
    }
    // SAFETY: kill with signal 0 only checks for existence.
    let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn mtime(p: &Path) -> Option<SystemTime> {
    fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn ms(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn read_dir(p: &Path) -> Vec<PathBuf> {
    fs::read_dir(p).map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect()).unwrap_or_default()
}

fn stem(p: &Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
}

/// Lines we never need, recognised from their first bytes so huge ones are not parsed.
fn skip_line(src: &Src, line: &[u8]) -> bool {
    let head = &line[..line.len().min(200)];
    let has = |pat: &[u8]| head.windows(pat.len()).any(|w| w == pat);
    match src {
        Src::Claude { .. } | Src::ClaudeSub { .. } => {
            has(b"\"type\":\"api-request") || has(b"\"type\":\"file-history") || has(b"\"type\":\"data-flow")
        }
        Src::Codex { .. } => {
            has(b"\"type\":\"compacted\"")
                || has(b"\"type\":\"world_state\"")
                || has(b"\"type\":\"reasoning\"")
                || has(b"\"type\":\"token_usage_record\"")
                || has(b"\"type\":\"additional_tools\"")
        }
        Src::Agy { .. } => has(b"\"type\":\"GENERIC\"") || has(b"\"type\": \"GENERIC\""),
        Src::Pi { .. } => false,
    }
}

impl Collector {
    pub fn new(lookback_hours: f64) -> Collector {
        Collector {
            host: self_host(),
            home: home_dir(),
            lookback_ms: (lookback_hours * 3_600_000.0) as i64,
            files: HashMap::new(),
            accs: HashMap::new(),
            inbox_seen: HashSet::new(),
            registry_seen: HashMap::new(),
            last_discover: None,
            msg_count: 0,
        }
    }

    pub fn roots(&self) -> Vec<String> {
        let mut out = vec![relay_home().display().to_string()];
        for r in [".claude/projects", ".codex/sessions", ".gemini/antigravity", ".gemini/antigravity-cli", ".gemini/antigravity-ide", ".pi/agent/sessions"] {
            let p = self.home.join(r);
            if p.is_dir() {
                out.push(p.display().to_string());
            }
        }
        out
    }

    fn cutoff(&self) -> SystemTime {
        SystemTime::now() - Duration::from_millis(self.lookback_ms as u64)
    }

    fn acc(&mut self, kind: AgentKind, id: &str) -> &mut Acc {
        let host = self.host.clone();
        self.accs.entry((kind, id.to_string())).or_insert_with(|| Acc::new(kind, id, &host))
    }

    fn consider(&mut self, path: PathBuf, src: Src) {
        if self.files.contains_key(&path) {
            return;
        }
        let cutoff = self.cutoff();
        if mtime(&path).is_some_and(|m| m >= cutoff) {
            self.files.insert(path, Tracked { src, offset: 0 });
        }
    }

    /// Find transcript files that changed inside the lookback window.
    fn discover(&mut self) {
        let home = self.home.clone();
        for proj in read_dir(&home.join(".claude/projects")) {
            for entry in read_dir(&proj) {
                if entry.extension().is_some_and(|e| e == "jsonl") {
                    let sid = stem(&entry);
                    self.consider(entry, Src::Claude { sid });
                } else if entry.is_dir() {
                    let sid = entry.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                    for sub in read_dir(&entry.join("subagents")) {
                        if sub.extension().is_some_and(|e| e == "jsonl") {
                            let aid = stem(&sub).trim_start_matches("agent-").to_string();
                            self.consider(sub, Src::ClaudeSub { sid: sid.clone(), aid });
                        }
                    }
                }
            }
        }
        let cutoff = self.cutoff();
        let codex = home.join(".codex/sessions");
        let mut stack = vec![(codex, 0)];
        while let Some((dir, depth)) = stack.pop() {
            for e in read_dir(&dir) {
                if e.is_dir() && depth < 4 {
                    // Date folders older than the window cannot hold new rollouts... but an old
                    // rollout can still be appended to, so check mtimes rather than names.
                    if mtime(&e).is_none_or(|m| m >= cutoff) || depth < 3 {
                        stack.push((e, depth + 1));
                    }
                } else if e.extension().is_some_and(|x| x == "jsonl") {
                    let name = stem(&e);
                    if name.len() >= 36 {
                        let tid = name[name.len() - 36..].to_string();
                        self.consider(e, Src::Codex { tid });
                    }
                }
            }
        }
        for product in ["antigravity", "antigravity-cli", "antigravity-ide"] {
            for conv in read_dir(&home.join(".gemini").join(product).join("brain")) {
                let logs = conv.join(".system_generated/logs");
                let t = logs.join("transcript.jsonl");
                let path = if t.is_file() { t } else { logs.join("transcript_full.jsonl") };
                if path.is_file() {
                    let cid = conv.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                    self.consider(path, Src::Agy { cid });
                }
            }
        }
        for dir in read_dir(&home.join(".pi/agent/sessions")) {
            for f in read_dir(&dir) {
                if f.extension().is_some_and(|x| x == "jsonl") {
                    let name = stem(&f);
                    let id = name.rsplit('_').next().unwrap_or(&name).to_string();
                    self.consider(f, Src::Pi { id });
                }
            }
        }
    }

    fn tail(&mut self) {
        let paths: Vec<PathBuf> = self.files.keys().cloned().collect();
        for path in paths {
            let (src, offset) = {
                let t = &self.files[&path];
                (t.src.clone(), t.offset)
            };
            let len = match fs::metadata(&path) {
                Ok(m) => m.len(),
                Err(_) => {
                    self.files.remove(&path);
                    continue;
                }
            };
            if len == offset {
                continue;
            }
            let start = if len < offset { 0 } else { offset };
            let Ok(mut f) = fs::File::open(&path) else { continue };
            if f.seek(SeekFrom::Start(start)).is_err() {
                continue;
            }
            let mut reader = BufReader::with_capacity(1 << 20, f);
            let mut pos = start;
            let mut buf = Vec::new();
            let (kind, id) = src.key();
            loop {
                buf.clear();
                let n = match reader.read_until(b'\n', &mut buf) {
                    Ok(n) => n,
                    Err(_) => break,
                };
                if n == 0 || buf.last() != Some(&b'\n') {
                    break; // EOF or a line still being written
                }
                pos += n as u64;
                if skip_line(&src, &buf) {
                    continue;
                }
                let Ok(v) = serde_json::from_slice::<Value>(&buf) else { continue };
                let acc = self.acc(kind, &id);
                if acc.agent.transcript.is_none() && !matches!(src, Src::ClaudeSub { .. }) {
                    acc.agent.transcript = Some(path.display().to_string());
                }
                match &src {
                    Src::Claude { .. } => claude::line(acc, &v),
                    Src::ClaudeSub { aid, .. } => claude::subagent_line(acc, aid, &v),
                    Src::Codex { .. } => codex::line(acc, &v),
                    Src::Agy { .. } => agy::line(acc, &v),
                    Src::Pi { .. } => pi::line(acc, &v),
                }
            }
            if let Some(t) = self.files.get_mut(&path) {
                t.offset = pos;
            }
        }
    }

    fn registry(&mut self) {
        let cutoff_ms = now_ms() - self.lookback_ms;
        for p in read_dir(&relay_home().join("sessions")) {
            if p.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let Some(m) = mtime(&p) else { continue };
            let Ok(text) = fs::read_to_string(&p) else { continue };
            let Ok(rec) = serde_json::from_str::<Value>(&text) else { continue };
            let Some(kind) = rec.get("agent").and_then(|x| x.as_str()).and_then(AgentKind::parse) else { continue };
            let Some(id) = rec.get("id").and_then(|x| x.as_str()) else { continue };
            let pid = rec.get("pid").and_then(|x| x.as_i64());
            let alive = pid.map(pid_alive);
            let known = self.accs.contains_key(&(kind, id.to_string()));
            // Skip old dead records that have no recent transcript.
            if !known && ms(m) < cutoff_ms && alive != Some(true) {
                continue;
            }
            let changed = self.registry_seen.get(&p) != Some(&m);
            self.registry_seen.insert(p.clone(), m);
            if let Some(tp) = rec.get("transcript_path").and_then(|x| x.as_str()) {
                let tp = PathBuf::from(tp);
                if tp.is_file() && !self.files.contains_key(&tp) {
                    let src = match kind {
                        AgentKind::Claude => Some(Src::Claude { sid: id.to_string() }),
                        AgentKind::Antigravity => Some(Src::Agy { cid: id.to_string() }),
                        _ => None,
                    };
                    if let Some(src) = src {
                        // Known sessions are read in full even when their transcript is older.
                        if alive == Some(true) || ms(m) >= cutoff_ms {
                            self.files.insert(tp, Tracked { src, offset: 0 });
                        }
                    }
                }
            }
            let acc = self.acc(kind, id);
            if acc.agent.alive != alive {
                acc.agent.alive = alive;
                acc.dirty = true;
            }
            if !changed {
                continue;
            }
            acc.agent.registered = true;
            if let Some(c) = rec.get("cwd").and_then(|x| x.as_str()) {
                acc.set_cwd(c);
            }
            if let Some(l) = rec.get("label").and_then(|x| x.as_str()) {
                acc.set_label(l, 2);
            }
            let state = rec.get("state").and_then(|x| x.as_str()).map(str::to_string);
            if acc.agent.state != state {
                acc.agent.state = state;
            }
            if acc.agent.last_active_ms < ms(m) {
                acc.agent.last_active_ms = ms(m);
            }
            acc.dirty = true;
        }
    }

    fn inbox(&mut self, emit: &mut dyn FnMut(CollectorEvent)) {
        let cutoff_ms = now_ms() - self.lookback_ms;
        let cutoff = self.cutoff();
        for sdir in read_dir(&relay_home().join("inbox")) {
            let sid = sdir.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let mut files = read_dir(&sdir);
            files.extend(read_dir(&sdir.join("delivered")));
            for f in files {
                if f.extension().is_none_or(|e| e != "json") || self.inbox_seen.contains(&f) {
                    continue;
                }
                if mtime(&f).is_none_or(|m| m < cutoff) {
                    self.inbox_seen.insert(f);
                    continue;
                }
                let Ok(text) = fs::read_to_string(&f) else { continue };
                self.inbox_seen.insert(f.clone());
                let Ok(env) = serde_json::from_str::<Value>(&text) else { continue };
                let g = |k: &str| env.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
                let sent_ms = env.get("sent_at_ms").and_then(|x| x.as_i64()).unwrap_or(0);
                if sent_ms < cutoff_ms {
                    continue;
                }
                let body = g("body");
                let bytes = env.get("body_chars").and_then(|x| x.as_u64()).unwrap_or(body.len() as u64);
                // The envelope's `to` has no host: it is this computer. The mailbox
                // folder holds the resolved session id.
                let kind = Addr::parse(&g("to")).map(|a| a.kind).unwrap_or_else(|| "claude".into());
                let to = format!("{kind}:{sid}@{}", self.host);
                self.msg_count += 1;
                emit(CollectorEvent::Message(RelayMessage {
                    id: g("id"),
                    from: g("from"),
                    to,
                    subject: env.get("subject").and_then(|x| x.as_str()).map(str::to_string),
                    body: truncate(&body, MAX_BODY_PREVIEW),
                    bytes,
                    sent_ms,
                    source: MsgSource::Inbox,
                    replaces: None,
                    from_agent: None,
                    to_agent: None,
                }));
            }
        }
    }

    fn flush(&mut self, emit: &mut dyn FnMut(CollectorEvent)) -> usize {
        let cutoff_ms = now_ms() - self.lookback_ms;
        let mut n = 0;
        for acc in self.accs.values_mut() {
            for m in acc.out_msgs.drain(..) {
                if m.sent_ms >= cutoff_ms {
                    self.msg_count += 1;
                    emit(CollectorEvent::Message(m));
                }
            }
            if !acc.dirty {
                continue;
            }
            acc.dirty = false;
            let recent = acc.agent.last_active_ms >= cutoff_ms || acc.agent.alive == Some(true);
            if recent {
                n += 1;
                emit(CollectorEvent::Agent(Box::new(acc.agent.clone())));
            }
        }
        n
    }

    /// One pass: discover (at most every 10 s), tail, registry, mailboxes, flush.
    pub fn scan(&mut self, emit: &mut dyn FnMut(CollectorEvent)) -> usize {
        if self.last_discover.is_none_or(|t| t.elapsed() > Duration::from_secs(10)) {
            self.discover();
            self.last_discover = Some(Instant::now());
        }
        self.registry();
        self.tail();
        self.inbox(emit);
        self.flush(emit)
    }

    /// Run forever, emitting events; stops when `emit` returns false.
    pub fn run(mut self, interval: Duration, emit: &mut dyn FnMut(CollectorEvent) -> bool) {
        let started = Instant::now();
        let ok = std::cell::Cell::new(true);
        let mut sink = |e: CollectorEvent| {
            if ok.get() && !emit(e) {
                ok.set(false);
            }
        };
        sink(CollectorEvent::Hello { host: self.host.clone(), version: env!("CARGO_PKG_VERSION").into(), roots: self.roots() });
        let agents = self.scan(&mut sink);
        let messages = self.msg_count;
        sink(CollectorEvent::Ready { agents, messages, millis: started.elapsed().as_millis() as u64 });
        while ok.get() {
            std::thread::sleep(interval);
            self.scan(&mut sink);
        }
    }
}
