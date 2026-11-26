//! Derived signals for an operator running many agents: health (working,
//! waiting, stuck, looping), token burn, estimated cost, and the attention list.

use crate::app::App;
use crate::util::*;
use relay_map_core::*;
use std::collections::{HashMap, VecDeque};

#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub enum Health {
    Looping,
    Stuck,
    Waiting,
    Working,
    Idle,
    Ended,
    Ghost,
}

impl Health {
    pub fn id(self) -> &'static str {
        match self {
            Health::Looping => "looping",
            Health::Stuck => "stuck",
            Health::Waiting => "waiting",
            Health::Working => "working",
            Health::Idle => "idle",
            Health::Ended => "ended",
            Health::Ghost => "ghost",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Health::Looping => "looping",
            Health::Stuck => "stuck",
            Health::Waiting => "waiting",
            Health::Working => "working",
            Health::Idle => "idle",
            Health::Ended => "ended",
            Health::Ghost => "messages only",
        }
    }
    pub fn color(self) -> Rgb {
        parse_hex(match self {
            Health::Looping => "#ff4fd8",
            Health::Stuck => "#ff5a36",
            Health::Waiting => "#ffd166",
            Health::Working => "#46f5a0",
            Health::Idle => "#6f86c4",
            Health::Ended => "#4f5a7a",
            Health::Ghost => "#4f5a7a",
        })
    }
    pub fn from_id(s: &str) -> Option<Health> {
        [Health::Looping, Health::Stuck, Health::Waiting, Health::Working, Health::Idle, Health::Ended, Health::Ghost]
            .into_iter()
            .find(|h| h.id() == s || (s == "active" && *h == Health::Working) || (s == "busy" && *h == Health::Working))
    }
}

#[derive(Clone)]
pub struct Insight {
    pub health: Health,
    /// Why the health is what it is, one short line each.
    pub reasons: Vec<String>,
    /// Recorded by the agent, or estimated from the model's list price.
    pub cost: Option<f64>,
    pub cost_estimated: bool,
    /// Fresh (input + output) tokens per minute over the recent observed span
    /// (0 when unknown). Cache traffic is counted apart below.
    pub burn_tpm: f64,
    /// Cache-write tokens per minute.
    pub burn_cw_tpm: f64,
    /// Cache-read tokens per minute.
    pub burn_cr_tpm: f64,
    /// Dollars per hour over the recent observed span.
    pub burn_usd_h: f64,
    pub idle_ms: i64,
    pub msgs: usize,
}

/// One observation of an agent's running totals.
#[derive(Clone, Copy)]
pub struct Sample {
    pub t: f64,
    pub tokens: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub cost: f64,
    pub edits: u64,
}

pub const HIST_MS: f64 = 30.0 * 60_000.0;

/// List prices per million tokens (input, output) for Claude models. Cache reads
/// are billed at 0.1x input and cache writes at 1.25x input. Other providers'
/// sessions show a cost only when the agent recorded one.
pub fn claude_price(model: &str) -> Option<(f64, f64)> {
    let m = model.to_lowercase();
    if !(m.contains("claude") || m.contains("opus") || m.contains("sonnet") || m.contains("haiku") || m.contains("fable") || m.contains("mythos")) {
        return None;
    }
    let has = |s: &str| m.contains(s);
    Some(if has("fable-5-2") || has("mythos-5-2") {
        (8.0, 40.0)
    } else if has("fable") || has("mythos") {
        (10.0, 50.0)
    } else if has("opus-5-5") {
        (4.0, 20.0)
    } else if has("opus-4-1") || has("opus-4-0") || has("opus-4-2025") || has("claude-3-opus") {
        (15.0, 75.0)
    } else if has("opus") {
        (5.0, 25.0)
    } else if has("sonnet-5") {
        (2.0, 10.0)
    } else if has("sonnet") {
        (3.0, 15.0)
    } else if has("haiku-5-5") {
        (0.1, 0.5)
    } else if has("haiku-4-5") {
        (1.0, 5.0)
    } else if has("haiku") {
        (0.8, 4.0)
    } else {
        return None;
    })
}

/// Estimated dollars by kind of token, from the model's list price.
#[derive(Clone, Copy, Default)]
pub struct CostParts {
    pub input: f64,
    pub output: f64,
    pub cache_write: f64,
    pub cache_read: f64,
}

impl CostParts {
    pub fn total(&self) -> f64 {
        self.input + self.output + self.cache_write + self.cache_read
    }
}

/// The list-price split of an agent's tokens, when its model has a known price.
pub fn cost_parts(a: &Agent) -> Option<CostParts> {
    let (pi, po) = a.model.as_deref().and_then(claude_price)?;
    let t = &a.tokens;
    Some(CostParts {
        input: t.input as f64 * pi / 1e6,
        output: t.output as f64 * po / 1e6,
        cache_write: t.cache_write as f64 * pi * 1.25 / 1e6,
        cache_read: t.cache_read as f64 * pi * 0.1 / 1e6,
    })
}

pub fn cost_of(a: &Agent) -> (Option<f64>, bool) {
    if let Some(c) = a.cost_usd.filter(|c| *c > 0.0) {
        return (Some(c), false);
    }
    match cost_parts(a) {
        Some(p) => (Some(p.total()), true),
        None => (None, false),
    }
}

pub fn edits_of(a: &Agent) -> u64 {
    a.files.iter().map(|f| f.count as u64).sum()
}

pub fn record_sample(hist: &mut HashMap<String, VecDeque<Sample>>, a: &Agent) {
    let t = now_ms();
    let s = Sample {
        t,
        tokens: a.tokens.fresh(),
        cache_write: a.tokens.cache_write,
        cache_read: a.tokens.cache_read,
        cost: cost_of(a).0.unwrap_or(0.0),
        edits: edits_of(a),
    };
    let q = hist.entry(a.addr.clone()).or_default();
    if let Some(last) = q.back() {
        if last.tokens == s.tokens && last.cache_write == s.cache_write && last.cache_read == s.cache_read && last.edits == s.edits && t - last.t < 20_000.0 {
            return;
        }
    }
    q.push_back(s);
    while q.front().map_or(false, |f| t - f.t > HIST_MS) && q.len() > 2 {
        q.pop_front();
    }
}

#[derive(Default)]
struct Rates {
    tpm: f64,
    cw_tpm: f64,
    cr_tpm: f64,
    usd_h: f64,
    edits: u64,
}

/// Fresh, cache-write and cache-read tokens per minute, dollars per hour and
/// edits over the last ten minutes of observations, with "now" counted as a
/// flat sample.
fn rates(q: Option<&VecDeque<Sample>>, last_active: i64) -> Rates {
    let Some(q) = q else { return Rates::default() };
    let (Some(first), Some(last)) = (q.iter().find(|s| now_ms() - s.t <= 10.0 * 60_000.0), q.back()) else { return Rates::default() };
    let edits = last.edits.saturating_sub(first.edits);
    // An agent that went quiet is not burning, however busy its last stretch was.
    let end = now_ms().max(last.t);
    let span = (end - first.t).max(1.0);
    if span < 30_000.0 || now_ms() as i64 - last_active > 10 * 60_000 {
        return Rates { edits, ..Rates::default() };
    }
    let per_min = |a: u64, b: u64| b.saturating_sub(a) as f64 / (span / 60_000.0);
    Rates {
        tpm: per_min(first.tokens, last.tokens),
        cw_tpm: per_min(first.cache_write, last.cache_write),
        cr_tpm: per_min(first.cache_read, last.cache_read),
        usd_h: (last.cost - first.cost).max(0.0) / (span / 3_600_000.0),
        edits,
    }
}

/// Per-minute fresh-token deltas for the last `bins` minutes (a sparkline).
pub fn burn_series(q: Option<&VecDeque<Sample>>, bins: usize) -> Vec<f64> {
    series(q, bins, |s| s.tokens)
}

/// Per-minute cache-read deltas for the last `bins` minutes.
pub fn cache_read_series(q: Option<&VecDeque<Sample>>, bins: usize) -> Vec<f64> {
    series(q, bins, |s| s.cache_read)
}

fn series(q: Option<&VecDeque<Sample>>, bins: usize, f: impl Fn(&Sample) -> u64) -> Vec<f64> {
    let mut out = vec![0.0; bins];
    let Some(q) = q else { return out };
    let now = now_ms();
    for w in q.iter().collect::<Vec<_>>().windows(2) {
        let (a, b) = (w[0], w[1]);
        let age = now - b.t;
        let i = (age / 60_000.0) as usize;
        if i < bins {
            out[bins - 1 - i] += f(b).saturating_sub(f(a)) as f64;
        }
    }
    out
}

pub fn compute(app: &App) -> HashMap<String, Insight> {
    let now = now_ms() as i64;
    // Ping-pong: two agents trading many relay messages in a short span.
    let mut pairs: HashMap<(String, String), (usize, usize)> = HashMap::new();
    let mut msgs: HashMap<String, usize> = HashMap::new();
    for m in app.messages.iter().filter(|m| now - m.sent_ms < app.window_ms) {
        if m.source == MsgSource::Swarm {
            continue;
        }
        for x in [m.from_agent.as_ref(), m.to_agent.as_ref()].into_iter().flatten() {
            *msgs.entry(x.clone()).or_default() += 1;
        }
        if now - m.sent_ms > 15 * 60_000 || m.source == MsgSource::Broadcast {
            continue;
        }
        let (Some(f), Some(t)) = (m.from_agent.clone(), m.to_agent.clone()) else { continue };
        if f == t {
            continue;
        }
        let fwd = f < t;
        let key = if fwd { (f, t) } else { (t, f) };
        let e = pairs.entry(key).or_default();
        if fwd {
            e.0 += 1
        } else {
            e.1 += 1
        }
    }
    let mut pingpong: HashMap<String, Vec<String>> = HashMap::new();
    for ((a, b), (ab, ba)) in &pairs {
        if ab + ba >= 8 && (*ab).min(*ba) >= 3 {
            let n = ab + ba;
            pingpong.entry(a.clone()).or_default().push(format!("ping-pong with {}: {n} messages in 15m", app.name(b)));
            pingpong.entry(b.clone()).or_default().push(format!("ping-pong with {}: {n} messages in 15m", app.name(a)));
        }
    }
    let mut out = HashMap::new();
    for (addr, a) in &app.agents {
        let idle = now - a.last_active_ms.max(0);
        let (cost, est) = cost_of(a);
        let r = rates(app.tok_hist.get(addr), a.last_active_ms);
        let edits10 = r.edits;
        let mut reasons = Vec::new();
        let mut loop_hit = pingpong.remove(addr).unwrap_or_default();
        if let Some(f) = a.files.iter().filter(|f| f.count >= 40 && now - f.at_ms < 20 * 60_000).max_by_key(|f| f.count) {
            loop_hit.push(format!("edited {} ×{} (last {})", short_path(&f.path, 2), f.count, ago(f.at_ms)));
        }
        if edits10 >= 15 && a.files.iter().filter(|f| now - f.at_ms < 10 * 60_000).count() <= 2 {
            loop_hit.push(format!("{edits10} edits to the same file or two in 10m"));
        }
        let health = if a.ghost {
            Health::Ghost
        } else if !loop_hit.is_empty() && idle < 20 * 60_000 {
            reasons.extend(loop_hit);
            Health::Looping
        } else if a.state.as_deref() == Some("busy") && idle > 15 * 60_000 && idle < 6 * 3_600_000 && a.alive != Some(false) {
            reasons.push(format!("marked busy but silent for {}", dur(idle)));
            Health::Stuck
        } else if idle < 2 * 60_000 {
            Health::Working
        } else if a.alive == Some(true) {
            reasons.push(format!("process alive, quiet for {}", dur(idle)));
            Health::Waiting
        } else if a.alive == Some(false) {
            Health::Ended
        } else {
            Health::Idle
        };
        out.insert(
            addr.clone(),
            Insight {
                health,
                reasons,
                cost,
                cost_estimated: est,
                burn_tpm: r.tpm,
                burn_cw_tpm: r.cw_tpm,
                burn_cr_tpm: r.cr_tpm,
                burn_usd_h: r.usd_h,
                idle_ms: idle, msgs: msgs.get(addr).copied().unwrap_or(0) },
        );
    }
    out
}

/// Something the operator should look at.
pub struct Alert {
    /// 0 = act now, 1 = look soon, 2 = for information.
    pub sev: u8,
    pub kind: &'static str,
    pub addr: Option<String>,
    pub text: String,
    pub at_ms: i64,
}

pub const HOT_USD_H: f64 = 30.0;

pub fn alerts(app: &App) -> Vec<Alert> {
    let now = now_ms() as i64;
    let mut v = Vec::new();
    for (addr, i) in &app.insights {
        if !app.drawn_as.contains_key(addr) {
            continue;
        }
        let a = &app.agents[addr];
        match i.health {
            Health::Looping => v.push(Alert { sev: 0, kind: "looping", addr: Some(addr.clone()), text: i.reasons.join("; "), at_ms: a.last_active_ms }),
            Health::Stuck => v.push(Alert { sev: 0, kind: "stuck", addr: Some(addr.clone()), text: i.reasons.join("; "), at_ms: a.last_active_ms }),
            // Past a few hours a quiet session is parked, not waiting on a reply.
            Health::Waiting if i.idle_ms > 10 * 60_000 && i.idle_ms < 3 * 3_600_000 => v.push(Alert {
                sev: 1,
                kind: "waiting",
                addr: Some(addr.clone()),
                text: format!("quiet for {}; may be waiting for you{}", dur(i.idle_ms), a.focus.as_deref().map(|f| format!(" · last: {}", truncate(f, 80))).unwrap_or_default()),
                at_ms: a.last_active_ms,
            }),
            _ => {}
        }
        if i.burn_usd_h >= HOT_USD_H {
            v.push(Alert { sev: 1, kind: "spend", addr: Some(addr.clone()), text: format!("spending about ${:.0}/h ({} tokens/min)", i.burn_usd_h, big(i.burn_tpm as u64)), at_ms: now });
        }
    }
    for m in app.messages.iter().rev().take_while(|m| now - m.sent_ms < 3_600_000) {
        if app.is_cross(m) {
            v.push(Alert {
                sev: 0,
                kind: "cross-pool",
                addr: m.from_agent.clone(),
                text: format!(
                    "{} → {} crossed an isolated pool: {}",
                    m.from_agent.as_deref().map(|x| app.name(x)).unwrap_or(m.from.clone()),
                    m.to_agent.as_deref().map(|x| app.name(x)).unwrap_or(m.to.clone()),
                    truncate(m.subject.as_deref().unwrap_or(&m.body), 60)
                ),
                at_ms: m.sent_ms,
            });
        }
    }
    for h in &app.hosts {
        if h.state == "error" {
            v.push(Alert { sev: 0, kind: "computer", addr: None, text: format!("{} unreachable: {}", h.name, truncate(&h.detail, 90)), at_ms: now });
        }
    }
    v.sort_by(|a, b| a.sev.cmp(&b.sev).then(b.at_ms.cmp(&a.at_ms)));
    v
}

pub fn dur(ms: i64) -> String {
    let s = (ms / 1000).max(0);
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m", s / 60),
        3600..=86399 => format!("{}h {}m", s / 3600, (s % 3600) / 60),
        _ => format!("{}d", s / 86400),
    }
}

pub fn usd(c: f64) -> String {
    if c >= 100.0 {
        format!("${c:.0}")
    } else if c >= 0.995 {
        format!("${c:.2}")
    } else {
        format!("{:.0}¢", (c * 100.0).max(0.0))
    }
}

/// A search query: free words plus `key:value` filters.
#[derive(Default)]
pub struct Query {
    pub words: Vec<String>,
    pub filters: Vec<(String, String)>,
}

impl Query {
    pub fn parse(s: &str) -> Query {
        let mut q = Query::default();
        for tok in s.split_whitespace() {
            let t = tok.to_lowercase();
            match t.split_once(':') {
                Some((k, v)) if !v.is_empty() && matches!(k, "kind" | "type" | "host" | "on" | "status" | "is" | "project" | "pool" | "model" | "has") => {
                    q.filters.push((k.to_string(), v.to_string()))
                }
                _ => q.words.push(t),
            }
        }
        q
    }
    pub fn is_empty(&self) -> bool {
        self.words.is_empty() && self.filters.is_empty()
    }
    pub fn matches(&self, a: &Agent, i: Option<&Insight>, name: &str) -> bool {
        for (k, v) in &self.filters {
            let ok = match k.as_str() {
                "kind" | "type" => a.kind.map_or("other", |k| k.as_str()).starts_with(v.as_str()) || (v == "openai" && a.kind == Some(AgentKind::Codex)),
                "host" | "on" => a.host.to_lowercase().contains(v.as_str()),
                "status" | "is" => i.map_or(false, |i| Some(i.health) == Health::from_id(v) || (v == "alert" && matches!(i.health, Health::Stuck | Health::Looping)) || (v == "hot" && i.burn_usd_h >= HOT_USD_H)),
                "project" => project_of(a.cwd.as_deref().unwrap_or("")).to_lowercase().contains(v.as_str()),
                "pool" => a.pool.as_deref().unwrap_or("").to_lowercase().contains(v.as_str()),
                "model" => a.model.as_deref().unwrap_or("").to_lowercase().contains(v.as_str()),
                "has" => match v.as_str() {
                    "git" => !a.git.is_empty(),
                    "files" => !a.files.is_empty(),
                    "subagents" | "subs" => !a.subagents.is_empty(),
                    "msgs" | "messages" => i.map_or(false, |i| i.msgs > 0),
                    "cost" => i.map_or(false, |i| i.cost.is_some()),
                    _ => true,
                },
                _ => true,
            };
            if !ok {
                return false;
            }
        }
        if self.words.is_empty() {
            return true;
        }
        let hay = [name, &a.display_name(), a.title.as_deref().unwrap_or(""), a.focus.as_deref().unwrap_or(""), a.cwd.as_deref().unwrap_or(""), &a.addr, a.model.as_deref().unwrap_or("")].join(" ").to_lowercase();
        self.words.iter().all(|w| hay.contains(w.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prices() {
        assert_eq!(claude_price("claude-opus-5-5[1m]"), Some((4.0, 20.0)));
        assert_eq!(claude_price("claude-sonnet-4-6"), Some((3.0, 15.0)));
        assert_eq!(claude_price("claude-haiku-5-5"), Some((0.1, 0.5)));
        assert_eq!(claude_price("gpt-5-codex"), None);
    }

    #[test]
    fn query_filters() {
        let a = Agent { kind: Some(AgentKind::Codex), host: "laptop".into(), cwd: Some("/home/u/git/widgets".into()), title: Some("Fix the parser".into()), ..Default::default() };
        assert!(Query::parse("kind:codex host:lap").matches(&a, None, "widgets"));
        assert!(Query::parse("project:widg parser").matches(&a, None, "widgets"));
        assert!(!Query::parse("kind:claude").matches(&a, None, "widgets"));
        assert!(!Query::parse("unrelated").matches(&a, None, "widgets"));
    }
}
