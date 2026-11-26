//! Front-end state: agents, messages, layout, camera, animations.

use crate::group::{self, Level};
use crate::insight::{self, Alert, Insight, Query, Sample};
use crate::util::*;
use relay_map_core::*;
use std::collections::{HashMap, HashSet, VecDeque};

pub const UNPOOLED: &str = "Unpooled";

pub struct Node {
    pub x: f64,
    pub y: f64,
    pub tx: f64,
    pub ty: f64,
    pub r: f64,
    /// Children folded into this node (swarm members).
    pub children: Vec<String>,
    pub born: f64,
}

pub struct Region {
    pub name: String,
    pub color: Rgb,
    pub x: f64,
    pub y: f64,
    pub r: f64,
    pub tr: f64,
    pub tx: f64,
    pub ty: f64,
    pub members: usize,
    pub isolated: bool,
    /// Text on the rim: the project for computer pools, else the pool name.
    pub label: String,
    pub host: Option<String>,
    /// Agent-type sectors: (kind, middle angle, count).
    pub kinds: Vec<(String, f64, usize)>,
    /// 1 for the outermost circles.
    pub depth: usize,
}

/// An arc behind the pools of one computer.
pub struct HostBand {
    pub host: String,
    pub a0: f64,
    pub a1: f64,
    pub radius: f64,
}

pub struct Pulse {
    pub from: String,
    pub to: String,
    pub t0: f64,
    pub dur: f64,
    pub size: f64,
    pub color: Rgb,
    pub cross: bool,
    pub internal: bool,
    pub swarm: bool,
}

pub struct Ripple {
    pub x: f64,
    pub y: f64,
    pub t0: f64,
    pub color: Rgb,
    pub size: f64,
}

#[derive(Clone)]
pub struct Channel {
    pub a: String,
    pub b: String,
    pub count: usize,
    pub bytes: u64,
    pub a_to_b: usize,
    pub last_ms: i64,
    pub cross: bool,
    pub swarm_only: bool,
    pub color: Rgb,
}

#[derive(Clone, PartialEq)]
pub enum Selection {
    None,
    Agent(String),
    Channel(String, String),
}

pub struct Camera {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
    pub tx: f64,
    pub ty: f64,
    pub tzoom: f64,
}

pub struct Replay {
    pub start_real: f64,
    pub span_real: f64,
    pub from_ms: i64,
    pub to_ms: i64,
    pub next: usize,
    pub queue: Vec<RelayMessage>,
}

pub struct App {
    pub self_host: String,
    pub agents: HashMap<String, Agent>,
    pub messages: Vec<RelayMessage>,
    pub msg_ids: HashSet<String>,
    pub pools: Vec<Pool>,
    pub hosts: Vec<HostStatus>,
    pub nodes: HashMap<String, Node>,
    pub regions: Vec<Region>,
    pub host_bands: Vec<HostBand>,
    pub pulses: Vec<Pulse>,
    pub ripples: Vec<Ripple>,
    pub channels: Vec<Channel>,
    /// Agent address -> the visible node it is drawn as (itself or its swarm parent).
    pub drawn_as: HashMap<String, String>,
    pub selection: Selection,
    pub hover: Option<String>,
    pub cam: Camera,
    pub width: f64,
    pub height: f64,
    pub dpr: f64,
    pub window_ms: i64,
    pub show_unpooled: bool,
    pub show_swarm: bool,
    pub expand_swarms: bool,
    pub search: String,
    pub replay: Option<Replay>,
    pub connected: bool,
    pub layout_dirty: bool,
    pub panel_dirty: bool,
    pub hud_dirty: bool,
    pub feed: Vec<RelayMessage>,
    pub broadcast_req: Option<String>,
    pub broadcast_results: Vec<BroadcastTarget>,
    pub broadcast_plan: Option<(Vec<String>, Vec<String>)>,
    pub drag: Option<(f64, f64, f64, f64, bool)>,
    pub first_snapshot: bool,
    pub last_layout: f64,
    /// Display names, made unique among visible agents.
    pub names: HashMap<String, String>,
    /// The mouse is over the feed: do not reshuffle it.
    pub feed_paused: bool,
    /// Grouping levels, outermost first.
    pub levels: Vec<Level>,
    /// Health, burn and cost per agent, refreshed every couple of seconds.
    pub insights: HashMap<String, Insight>,
    pub alerts: Vec<Alert>,
    /// Running totals observed since the page opened, for burn rates.
    pub tok_hist: HashMap<String, VecDeque<Sample>>,
    pub query: Query,
    pub color_by: ColorBy,
    /// Agents the operator is watching (starred).
    pub pins: HashSet<String>,
    pub drawer: Option<Tab>,
    /// Agents table sort column and direction.
    pub sort: (String, bool),
    pub last_insight: f64,
    /// Which alert `n` jumps to next.
    pub alert_cursor: usize,
    pub drawer_dirty: bool,
    /// Messages expanded in the messages log.
    pub open_msgs: HashSet<String>,
    /// The mouse is over the drawer: hold its rows still.
    pub drawer_hover: bool,
    /// A channel to highlight while the mouse is over a message that crossed it.
    pub hover_channel: Option<(String, String)>,
    /// Desktop notifications for new stuck, looping or waiting agents.
    pub notify: bool,
    /// Health last seen per agent, to notice changes.
    pub prev_health: HashMap<String, crate::insight::Health>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ColorBy {
    Kind,
    Health,
    Spend,
}

impl ColorBy {
    pub fn id(self) -> &'static str {
        match self {
            ColorBy::Kind => "kind",
            ColorBy::Health => "health",
            ColorBy::Spend => "spend",
        }
    }
    pub fn from_id(s: &str) -> ColorBy {
        match s {
            "health" => ColorBy::Health,
            "spend" => ColorBy::Spend,
            _ => ColorBy::Kind,
        }
    }
    pub fn next(self) -> ColorBy {
        match self {
            ColorBy::Kind => ColorBy::Health,
            ColorBy::Health => ColorBy::Spend,
            ColorBy::Spend => ColorBy::Kind,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Attention,
    Agents,
    Messages,
    Spend,
}

impl Tab {
    pub fn id(self) -> &'static str {
        match self {
            Tab::Attention => "attention",
            Tab::Agents => "agents",
            Tab::Messages => "messages",
            Tab::Spend => "spend",
        }
    }
    pub fn from_id(s: &str) -> Option<Tab> {
        [Tab::Attention, Tab::Agents, Tab::Messages, Tab::Spend].into_iter().find(|t| t.id() == s)
    }
}

/// Blue → amber → red by how hot a value is on a log scale between lo and hi.
pub fn heat(v: f64, lo: f64, hi: f64) -> Rgb {
    if v <= 0.0 {
        return parse_hex("#3c4b78");
    }
    let k = ((v.max(lo).ln() - lo.ln()) / (hi.ln() - lo.ln())).clamp(0.0, 1.0);
    let stops = [(0.0, (76.0, 184.0, 255.0)), (0.5, (255.0, 209.0, 102.0)), (1.0, (255.0, 59.0, 92.0))];
    let (a, b) = if k < 0.5 { (stops[0], stops[1]) } else { (stops[1], stops[2]) };
    let u = (k - a.0) / (b.0 - a.0);
    Rgb(a.1 .0 + (b.1 .0 - a.1 .0) * u, a.1 .1 + (b.1 .1 - a.1 .1) * u, a.1 .2 + (b.1 .2 - a.1 .2) * u)
}

pub fn kind_color(k: Option<AgentKind>) -> Rgb {
    match k {
        Some(AgentKind::Claude) => parse_hex("#ff8a4c"),
        Some(AgentKind::Codex) => parse_hex("#46f5a0"),
        Some(AgentKind::Antigravity) => parse_hex("#4cb8ff"),
        Some(AgentKind::Opencode) => parse_hex("#2ee6d6"),
        Some(AgentKind::Pi) => parse_hex("#c78bff"),
        Some(AgentKind::Operator) => parse_hex("#ffffff"),
        None => parse_hex("#8892b0"),
    }
}

impl App {
    pub fn new() -> App {
        App {
            self_host: String::new(),
            agents: HashMap::new(),
            messages: Vec::new(),
            msg_ids: HashSet::new(),
            pools: Vec::new(),
            hosts: Vec::new(),
            nodes: HashMap::new(),
            regions: Vec::new(),
            host_bands: Vec::new(),
            pulses: Vec::new(),
            ripples: Vec::new(),
            channels: Vec::new(),
            drawn_as: HashMap::new(),
            selection: Selection::None,
            hover: None,
            cam: Camera { x: 0.0, y: 0.0, zoom: 0.55, tx: 0.0, ty: 0.0, tzoom: 0.55 },
            width: 1200.0,
            height: 800.0,
            dpr: 1.0,
            window_ms: 24 * 3600 * 1000,
            show_unpooled: true,
            show_swarm: true,
            expand_swarms: false,
            search: String::new(),
            replay: None,
            connected: false,
            layout_dirty: true,
            panel_dirty: true,
            hud_dirty: true,
            feed: Vec::new(),
            broadcast_req: None,
            broadcast_results: Vec::new(),
            broadcast_plan: None,
            drag: None,
            first_snapshot: true,
            last_layout: 0.0,
            names: HashMap::new(),
            feed_paused: false,
            levels: vec![Level::Host, Level::Project, Level::Directory, Level::Task],
            insights: HashMap::new(),
            alerts: Vec::new(),
            tok_hist: HashMap::new(),
            query: Query::default(),
            color_by: ColorBy::Kind,
            pins: HashSet::new(),
            drawer: None,
            sort: ("active".into(), true),
            last_insight: 0.0,
            alert_cursor: 0,
            drawer_dirty: true,
            open_msgs: HashSet::new(),
            drawer_hover: false,
            hover_channel: None,
            notify: false,
            prev_health: HashMap::new(),
        }
    }

    pub fn operator(&self) -> String {
        format!("operator:relay-map@{}", self.self_host)
    }

    pub fn pool_color(&self, name: Option<&str>) -> Rgb {
        match name.and_then(|n| self.pools.iter().find(|p| p.name == n)) {
            Some(p) => parse_hex(&p.color),
            None => parse_hex("#8892b0"),
        }
    }

    pub fn pool_isolated(&self, name: Option<&str>) -> bool {
        name.and_then(|n| self.pools.iter().find(|p| p.name == n)).map_or(false, |p| p.isolated)
    }

    /// A message crosses isolated pools when both ends are pooled differently
    /// and at least one of those pools is isolated.
    pub fn is_cross(&self, m: &RelayMessage) -> bool {
        // Swarm traffic stays inside one orchestrator's family, even when the other
        // conversation's transcript was not collected.
        if m.source == MsgSource::Swarm {
            return false;
        }
        let pa = m.from_agent.as_ref().and_then(|a| self.agents.get(a)).and_then(|a| a.pool.clone());
        let pb = m.to_agent.as_ref().and_then(|a| self.agents.get(a)).and_then(|a| a.pool.clone());
        match (pa, pb) {
            (Some(a), Some(b)) if a != b => self.pool_isolated(Some(&a)) || self.pool_isolated(Some(&b)),
            _ => false,
        }
    }

    pub fn ingest(&mut self, msg: ServerMsg) {
        match msg {
            ServerMsg::Snapshot { now_ms: _, self_host, agents, messages, pools, hosts } => {
                self.self_host = self_host;
                for a in &agents {
                    if hosts.iter().any(|x| x.name == a.host && x.state == "live") {
                        insight::record_sample(&mut self.tok_hist, a);
                    }
                }
                self.agents = agents.into_iter().map(|a| (a.addr.clone(), a)).collect();
                self.msg_ids = messages.iter().map(|m| m.id.clone()).collect();
                self.messages = messages;
                self.pools = pools;
                self.hosts = hosts;
                self.feed = self.messages.iter().rev().filter(|m| m.source != MsgSource::Swarm).take(14).cloned().collect();
                self.layout_dirty = true;
                self.panel_dirty = true;
                self.hud_dirty = true;
                if self.first_snapshot {
                    self.first_snapshot = false;
                    self.start_replay(36_000.0);
                }
            }
            ServerMsg::Agent(a) => {
                let a = *a;
                let is_new = !self.agents.contains_key(&a.addr);
                let host_live = self.hosts.iter().any(|x| x.name == a.host && x.state == "live");
                if !host_live {
                    self.tok_hist.remove(&a.addr);
                }
                insight::record_sample(&mut self.tok_hist, &a);
                self.drawer_dirty = true;
                if self.selection == Selection::Agent(a.addr.clone()) {
                    self.panel_dirty = true;
                }
                self.agents.insert(a.addr.clone(), a);
                if is_new {
                    self.layout_dirty = true;
                }
                self.hud_dirty = true;
            }
            ServerMsg::Message(m) => self.add_message(m, true),
            ServerMsg::Pools { pools } => {
                self.pools = pools;
                self.layout_dirty = true;
                self.hud_dirty = true;
            }
            ServerMsg::Host(h) => {
                // A computer that just finished its backfill delivered whole histories at
                // once; restart burn rates from here so the jump does not count as burn.
                let was_live = self.hosts.iter().any(|x| x.name == h.name && x.state == "live");
                if h.state == "live" && !was_live {
                    for a in self.agents.values().filter(|a| a.host == h.name) {
                        if let Some(q) = self.tok_hist.get_mut(&a.addr) {
                            let last = q.back().copied();
                            q.clear();
                            q.extend(last);
                        }
                    }
                }
                if let Some(x) = self.hosts.iter_mut().find(|x| x.name == h.name) {
                    *x = h;
                } else {
                    self.hosts.push(h);
                }
                self.hud_dirty = true;
            }
            ServerMsg::BroadcastPlan { request_id, targets, skipped } => {
                if self.broadcast_req.as_deref() == Some(request_id.as_str()) {
                    self.broadcast_plan = Some((targets, skipped));
                    crate::ui::render_broadcast(self);
                }
            }
            ServerMsg::BroadcastResult { request_id, target, done: _ } => {
                if self.broadcast_req.as_deref() == Some(request_id.as_str()) {
                    self.broadcast_results.push(target);
                    crate::ui::render_broadcast(self);
                }
            }
            ServerMsg::Error { text } => crate::ui::toast(&text),
        }
    }

    fn add_message(&mut self, m: RelayMessage, live: bool) {
        if let Some(stub) = &m.replaces {
            if self.msg_ids.remove(stub) {
                self.messages.retain(|x| &x.id != stub);
                self.feed.retain(|x| &x.id != stub);
                // Its pulse was already shown; do not animate the delivered copy again.
                self.msg_ids.insert(m.id.clone());
                self.messages.push(m);
                self.layout_dirty = true;
                return;
            }
        }
        if self.msg_ids.contains(&m.id) {
            if let Some(x) = self.messages.iter_mut().find(|x| x.id == m.id) {
                let ends_changed = x.from_agent != m.from_agent || x.to_agent != m.to_agent;
                *x = m;
                if ends_changed {
                    self.layout_dirty = true;
                }
            }
            return;
        }
        self.msg_ids.insert(m.id.clone());
        let pos = self.messages.partition_point(|x| x.sent_ms <= m.sent_ms);
        self.messages.insert(pos, m.clone());
        if live && now_ms() as i64 - m.sent_ms < 10 * 60 * 1000 {
            self.launch(&m);
            if m.source != MsgSource::Swarm {
                self.feed.insert(0, m);
                self.feed.truncate(14);
                self.hud_dirty = true;
            }
        }
        self.layout_dirty = true;
        if let Selection::Agent(_) | Selection::Channel(..) = self.selection {
            self.panel_dirty = true;
        }
    }

    pub fn launch(&mut self, m: &RelayMessage) {
        if m.source == MsgSource::Swarm && !self.show_swarm {
            return;
        }
        let (Some(fa), Some(ta)) = (m.from_agent.as_ref(), m.to_agent.as_ref()) else { return };
        let (Some(f), Some(t)) = (self.drawn_as.get(fa).cloned(), self.drawn_as.get(ta).cloned()) else { return };
        let (Some(nf), Some(nt)) = (self.nodes.get(&f), self.nodes.get(&t)) else { return };
        let dist = ((nf.x - nt.x).powi(2) + (nf.y - nt.y).powi(2)).sqrt();
        let size = (2.0 + 1.7 * (1.0 + m.bytes as f64 / 150.0).log2()).clamp(2.5, 16.0);
        let cross = self.is_cross(m);
        let color = if cross {
            parse_hex("#ff3b5c")
        } else if m.source == MsgSource::Broadcast {
            parse_hex("#ffffff")
        } else if m.source == MsgSource::Swarm {
            parse_hex("#7fd4ff")
        } else {
            let pa = self.agents.get(fa).and_then(|a| a.pool.clone());
            if pa.is_some() { self.pool_color(pa.as_deref()) } else { kind_color(self.agents.get(fa).and_then(|a| a.kind)) }
        };
        let internal = f == t;
        self.pulses.push(Pulse {
            from: f,
            to: t,
            t0: now_ms(),
            dur: if internal { 900.0 } else { 900.0 + dist * 1.4 },
            size,
            color,
            cross,
            internal,
            swarm: m.source == MsgSource::Swarm,
        });
        if self.pulses.len() > 400 {
            self.pulses.remove(0);
        }
    }

    pub fn start_replay(&mut self, span_real: f64) {
        self.start_replay_from(now_ms() as i64 - self.window_ms, span_real);
    }

    pub fn start_replay_from(&mut self, from: i64, span_real: f64) {
        let now = now_ms() as i64;
        let queue: Vec<RelayMessage> = self
            .messages
            .iter()
            .filter(|m| m.sent_ms >= from && (self.show_swarm || m.source != MsgSource::Swarm))
            .cloned()
            .collect();
        if queue.is_empty() {
            self.replay = None;
            return;
        }
        let first = queue.first().map(|m| m.sent_ms).unwrap_or(from);
        self.replay = Some(Replay { start_real: now_ms(), span_real, from_ms: first, to_ms: now, next: 0, queue });
    }

    /// Advance the replay clock and launch due messages. Returns the simulated time.
    pub fn tick_replay(&mut self) -> Option<i64> {
        let r = self.replay.as_mut()?;
        let frac = ((now_ms() - r.start_real) / r.span_real).clamp(0.0, 1.0);
        let sim = r.from_ms + ((r.to_ms - r.from_ms) as f64 * frac) as i64;
        let mut due = Vec::new();
        while r.next < r.queue.len() && r.queue[r.next].sent_ms <= sim {
            due.push(r.queue[r.next].clone());
            r.next += 1;
        }
        let finished = frac >= 1.0;
        for m in due.iter().rev().take(60).rev() {
            self.launch(m);
        }
        if let Some(m) = due.iter().rev().find(|m| m.source != MsgSource::Swarm) {
            if !self.feed.iter().any(|f| f.id == m.id) {
                self.feed.insert(0, m.clone());
                self.feed.truncate(14);
                self.hud_dirty = true;
            }
        }
        if finished {
            self.replay = None;
        }
        Some(sim)
    }

    /// Recompute which agents are shown, regions, node targets and channels.
    pub fn layout(&mut self) {
        self.layout_dirty = false;
        self.last_layout = now_ms();
        let now = now_ms() as i64;
        let op = self.operator();
        // Agents with traffic in the window.
        let mut talked: HashSet<String> = HashSet::new();
        for m in self.messages.iter().filter(|m| now - m.sent_ms < self.window_ms) {
            if m.source == MsgSource::Swarm && !self.show_swarm {
                continue;
            }
            if let Some(a) = &m.from_agent {
                talked.insert(a.clone());
            }
            if let Some(a) = &m.to_agent {
                talked.insert(a.clone());
            }
        }
        let visible: HashSet<String> = self
            .agents
            .values()
            .filter(|a| {
                if a.addr == op {
                    return true;
                }
                let active = (!a.ghost && (a.alive == Some(true) || now - a.last_active_ms < self.window_ms)) || talked.contains(&a.addr);
                let configured = a.pool.as_deref().map_or(false, |n| self.pools.iter().any(|p| p.name == n && !p.auto));
                let pool_ok = self.show_unpooled || configured;
                active && pool_ok
            })
            .map(|a| a.addr.clone())
            .collect();
        // Fold swarm children into their nearest visible ancestor.
        self.drawn_as.clear();
        let mut children: HashMap<String, Vec<String>> = HashMap::new();
        for addr in &visible {
            let mut cur = addr.clone();
            if !self.expand_swarms {
                let mut hops = 0;
                while let Some(p) = self.agents.get(&cur).and_then(|a| a.parent.clone()) {
                    if !self.agents.contains_key(&p) || hops > 8 {
                        break;
                    }
                    cur = p;
                    hops += 1;
                }
            }
            if &cur != addr {
                children.entry(cur.clone()).or_default().push(addr.clone());
            }
            self.drawn_as.insert(addr.clone(), cur);
        }
        // Ancestors that are not visible themselves still need a node.
        let mut top: Vec<String> = self.drawn_as.values().cloned().collect::<HashSet<_>>().into_iter().collect();
        for t in &top {
            self.drawn_as.insert(t.clone(), t.clone());
        }
        top.sort();
        // Nested groups in the order the viewer chose (computer → project → …).
        let levels = self.levels.clone();
        let pools = self.pools.clone();
        let now = now_ms() as i64;
        let members: Vec<String> = top.iter().filter(|a| *a != &op).cloned().collect();
        let agents = &self.agents;
        let insights = &self.insights;
        let keyf = |l: Level, addr: &str| match (l, insights.get(addr)) {
            // Status groups follow the same health the badges and filters use.
            (Level::Status, Some(i)) => (format!("{}", i.health as u8), i.health.label().to_string(), Some(i.health.color())),
            _ => group::key(l, &agents[addr], &pools, now),
        };
        let node_r = |addr: &str| -> f64 {
            let a = &agents[addr];
            (7.0 + 2.4 * (1.0 + node_weight(&a.tokens) / 20000.0).log10()).clamp(7.0, 22.0)
        };
        let mut root = group::build(members, &levels, &keyf, String::new(), None, None);
        group::pack(&mut root, &node_r);
        // Top-level groups go around the operator in the middle.
        if root.children.is_empty() {
            root.x = 0.0;
            root.y = root.r + 90.0;
        } else {
            let mut order: Vec<usize> = (0..root.children.len()).collect();
            order.sort_by(|&a, &b| root.children[b].r.partial_cmp(&root.children[a].r).unwrap());
            let mut placed = vec![(0.0, 0.0, 70.0)];
            for &i in &order {
                let r = root.children[i].r;
                let (x, y) = group::find_spot(&placed, r, 40.0);
                root.children[i].x = x;
                root.children[i].y = y;
                placed.push((x, y, r));
            }
        }
        let golden = std::f64::consts::PI * (3.0 - 5f64.sqrt());
        let mut out_regions: Vec<(String, String, Option<Rgb>, usize, f64, f64, f64, usize, bool)> = Vec::new();
        let mut spots: Vec<(String, f64, f64)> = Vec::new();
        fn walk(
            n: &group::GNode,
            ox: f64,
            oy: f64,
            depth: usize,
            path: &str,
            pools: &[Pool],
            golden: f64,
            out: &mut Vec<(String, String, Option<Rgb>, usize, f64, f64, f64, usize, bool)>,
            spots: &mut Vec<(String, f64, f64)>,
        ) -> usize {
            let (x, y) = (ox + n.x, oy + n.y);
            let key = format!("{path}›{}", n.label);
            let mut count = n.agents.len();
            let idx = out.len();
            // A lone agent at the innermost level needs no circle of its own.
            let lone = depth > 1 && n.children.is_empty() && n.agents.len() == 1;
            if depth > 0 && !lone {
                let isolated = n.level == Some(Level::Pool) && pools.iter().any(|p| p.name == n.label && p.isolated);
                out.push((key.clone(), n.label.clone(), n.color, depth, x, y, n.r, 0, isolated));
            }
            let k = n.agents.len() as f64;
            for (j, a) in n.agents.iter().enumerate() {
                let rr = if k <= 1.0 { 0.0 } else { (n.r - 20.0) * ((j as f64 + 0.5) / k).sqrt() };
                let th = j as f64 * golden + (hash(&key) % 360) as f64;
                spots.push((a.clone(), x + rr * th.cos(), y + rr * th.sin()));
            }
            for c in &n.children {
                count += walk(c, x, y, depth + 1, &key, pools, golden, out, spots);
            }
            if depth > 0 && !lone {
                out[idx].7 = count;
            }
            count
        }
        walk(&root, 0.0, 0.0, 0, "", &pools, golden, &mut out_regions, &mut spots);
        for (a, x, y) in spots {
            self.place(&a, x, y);
        }
        let mut regions = Vec::new();
        for (key, label, color, depth, x, y, r, members, isolated) in out_regions {
            let old = self.regions.iter().find(|o| o.name == key);
            let neutral = [parse_hex("#7c8db5"), parse_hex("#9aa7d6"), parse_hex("#b6c1e6"), parse_hex("#cdd6f4")];
            regions.push(Region {
                name: key,
                color: color.unwrap_or(neutral[(depth - 1).min(3)]),
                x: old.map_or(x, |o| o.x),
                y: old.map_or(y, |o| o.y),
                r: old.map_or(r, |o| o.r),
                tr: r,
                tx: x,
                ty: y,
                members,
                isolated,
                label,
                host: None,
                kinds: Vec::new(),
                depth,
            });
        }
        self.host_bands.clear();
        self.regions = regions;
        self.place(&op.clone(), 0.0, 0.0);
        let keep: HashSet<String> = top.iter().cloned().collect();
        self.nodes.retain(|k, _| keep.contains(k));
        for (k, n) in self.nodes.iter_mut() {
            n.children = children.remove(k).unwrap_or_default();
            let a = &self.agents[k];
            let tok = node_weight(&a.tokens);
            n.r = if a.kind == Some(AgentKind::Operator) { 16.0 } else { (7.0 + 2.4 * (1.0 + tok / 20000.0).log10()).clamp(7.0, 22.0) };
            n.r += (n.children.len() as f64).sqrt().min(6.0);
        }
        self.compute_names();
        self.compute_channels();
        self.refresh_insights();
    }

    pub fn refresh_insights(&mut self) {
        self.last_insight = now_ms();
        self.insights = insight::compute(self);
        self.alerts = insight::alerts(self);
        self.notify_changes();
        self.drawer_dirty = true;
    }

    /// Raise a desktop notification when an agent turns stuck or looping, or
    /// starts waiting for the operator. Nothing fires for the first readings.
    fn notify_changes(&mut self) {
        use crate::insight::Health;
        let first = self.prev_health.is_empty();
        let mut fresh: Vec<(String, Health)> = Vec::new();
        for (addr, i) in &self.insights {
            if !self.drawn_as.contains_key(addr) {
                continue;
            }
            let before = self.prev_health.get(addr).copied();
            if !first && before != Some(i.health) && matches!(i.health, Health::Stuck | Health::Looping | Health::Waiting) {
                // A waiting agent only counts once it has been quiet a while, and not again until it works.
                if i.health != Health::Waiting || i.idle_ms < 3 * 3_600_000 {
                    fresh.push((addr.clone(), i.health));
                }
            }
        }
        self.prev_health = self.insights.iter().map(|(k, i)| (k.clone(), i.health)).collect();
        if self.notify {
            for (addr, h) in fresh.into_iter().take(4) {
                let why = self.insights.get(&addr).map(|i| i.reasons.join("; ")).unwrap_or_default();
                crate::notify::send(&format!("{} is {}", self.name(&addr), h.label()), &why, &addr);
            }
        }
    }

    /// Screen space the drawer covers on the left and the agent panel on the right.
    pub fn insets(&self) -> (f64, f64) {
        let left = if self.drawer.is_some() { self.width.min(860.0) } else { 0.0 };
        let right = if self.selection != Selection::None { self.width.min(500.0) } else { 0.0 };
        (left, right)
    }

    pub fn insight(&self, addr: &str) -> Option<&Insight> {
        self.insights.get(addr)
    }

    /// Visible agents that match the search, most recently active first.
    pub fn ordered_matches(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .nodes
            .keys()
            .filter(|k| self.agents.get(*k).map_or(false, |a| a.kind != Some(AgentKind::Operator) && self.matches_search(a)))
            .cloned()
            .collect();
        v.sort_by_key(|k| std::cmp::Reverse(self.agents[k].last_active_ms));
        v
    }

    /// The fill colour of a node under the current colour mode.
    pub fn node_color(&self, addr: &str) -> Rgb {
        let a = &self.agents[addr];
        if a.kind == Some(AgentKind::Operator) {
            return kind_color(a.kind);
        }
        match (self.color_by, self.insights.get(addr)) {
            (ColorBy::Health, Some(i)) => i.health.color(),
            (ColorBy::Spend, Some(i)) => match i.cost {
                Some(c) => heat(c, 0.5, 200.0),
                None => heat(a.tokens.fresh() as f64 / 1e6, 1.0, 400.0),
            },
            _ => kind_color(a.kind),
        }
    }

    pub fn name(&self, addr: &str) -> String {
        self.names
            .get(addr)
            .cloned()
            .or_else(|| self.agents.get(addr).map(|a| a.display_name()))
            .unwrap_or_else(|| addr.to_string())
    }

    /// Agents in the same folder share a base name; add a few words of their task.
    fn compute_names(&mut self) {
        let mut base: HashMap<String, Vec<String>> = HashMap::new();
        for addr in self.drawn_as.keys() {
            if let Some(a) = self.agents.get(addr) {
                base.entry(a.display_name()).or_default().push(addr.clone());
            }
        }
        self.names.clear();
        for (name, addrs) in base {
            let dup = addrs.len() > 1;
            for addr in addrs {
                let a = &self.agents[&addr];
                let n = if dup && a.label.is_none() {
                    match a.title.as_deref().filter(|t| !t.is_empty()) {
                        Some(t) => {
                            let words: Vec<&str> = t.split_whitespace().take(3).collect();
                            format!("{name} · {}", truncate(&words.join(" "), 22))
                        }
                        None => format!("{name} {}", a.id.chars().take(6).collect::<String>()),
                    }
                } else {
                    name.clone()
                };
                self.names.insert(addr, n);
            }
        }
    }

    fn place(&mut self, addr: &str, x: f64, y: f64) {
        let born = now_ms();
        let n = self.nodes.entry(addr.to_string()).or_insert(Node { x, y, tx: x, ty: y, r: 8.0, children: vec![], born });
        n.tx = x;
        n.ty = y;
    }

    pub fn compute_channels(&mut self) {
        let now = now_ms() as i64;
        let mut map: HashMap<(String, String), Channel> = HashMap::new();
        for m in self.messages.iter().filter(|m| now - m.sent_ms < self.window_ms) {
            if m.source == MsgSource::Swarm && !self.show_swarm {
                continue;
            }
            let (Some(fa), Some(ta)) = (m.from_agent.as_ref(), m.to_agent.as_ref()) else { continue };
            let (Some(f), Some(t)) = (self.drawn_as.get(fa), self.drawn_as.get(ta)) else { continue };
            if f == t {
                continue;
            }
            let (a, b, fwd) = if f < t { (f.clone(), t.clone(), true) } else { (t.clone(), f.clone(), false) };
            let cross = self.is_cross(m);
            let e = map.entry((a.clone(), b.clone())).or_insert(Channel {
                a,
                b,
                count: 0,
                bytes: 0,
                a_to_b: 0,
                last_ms: 0,
                cross: false,
                swarm_only: true,
                color: parse_hex("#9fb4ff"),
            });
            e.count += 1;
            e.bytes += m.bytes;
            e.last_ms = e.last_ms.max(m.sent_ms);
            e.cross |= cross;
            if fwd {
                e.a_to_b += 1;
            }
            if m.source != MsgSource::Swarm {
                e.swarm_only = false;
            }
        }
        let mut chans: Vec<Channel> = map.into_values().collect();
        for c in chans.iter_mut() {
            let pa = self.agents.get(&c.a).and_then(|x| x.pool.clone());
            let pb = self.agents.get(&c.b).and_then(|x| x.pool.clone());
            c.color = if c.cross {
                parse_hex("#ff3b5c")
            } else if pa.is_some() && pa == pb {
                self.pool_color(pa.as_deref())
            } else if c.swarm_only {
                parse_hex("#7fd4ff")
            } else {
                parse_hex("#c8d3ff")
            };
        }
        chans.sort_by_key(|c| c.count);
        self.channels = chans;
    }

    pub fn messages_between(&self, a: &str, b: &str) -> Vec<&RelayMessage> {
        self.messages
            .iter()
            .filter(|m| {
                let f = m.from_agent.as_ref().and_then(|x| self.drawn_as.get(x).or(Some(x)));
                let t = m.to_agent.as_ref().and_then(|x| self.drawn_as.get(x).or(Some(x)));
                matches!((f, t), (Some(f), Some(t)) if (f == a && t == b) || (f == b && t == a))
            })
            .collect()
    }

    pub fn messages_of(&self, addr: &str) -> Vec<&RelayMessage> {
        self.messages
            .iter()
            .filter(|m| m.from_agent.as_deref() == Some(addr) || m.to_agent.as_deref() == Some(addr))
            .collect()
    }

    pub fn world_to_screen(&self, x: f64, y: f64) -> (f64, f64) {
        ((x - self.cam.x) * self.cam.zoom + self.width / 2.0, (y - self.cam.y) * self.cam.zoom + self.height / 2.0)
    }

    pub fn screen_to_world(&self, x: f64, y: f64) -> (f64, f64) {
        ((x - self.width / 2.0) / self.cam.zoom + self.cam.x, (y - self.height / 2.0) / self.cam.zoom + self.cam.y)
    }

    pub fn fit_all(&mut self) {
        let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for r in &self.regions {
            minx = minx.min(r.tx - r.tr);
            maxx = maxx.max(r.tx + r.tr);
            miny = miny.min(r.ty - r.tr - 30.0);
            maxy = maxy.max(r.ty + r.tr);
        }
        if minx == f64::MAX {
            minx = -200.0;
            maxx = 200.0;
            miny = -200.0;
            maxy = 200.0;
        }
        // Room for the computer arcs and their labels outside the pools.
        let extra = 0.0;
        let w = (maxx - minx).max(200.0) + 160.0 + extra;
        let h = (maxy - miny).max(200.0) + 220.0 + extra;
        let (li, _) = self.insets();
        let avail = (self.width - li).max(200.0);
        self.cam.tzoom = (avail / w).min(self.height / h).clamp(0.15, 1.6);
        // Centre in the part of the screen the drawer leaves free.
        self.cam.tx = (minx + maxx) / 2.0 - (li / 2.0) / self.cam.tzoom;
        // The header covers the top of the screen.
        self.cam.ty = (miny + maxy) / 2.0 - 30.0 / self.cam.tzoom;
    }

    pub fn focus(&mut self, addr: &str) {
        if let Some(n) = self.nodes.get(self.drawn_as.get(addr).map(|s| s.as_str()).unwrap_or(addr)) {
            self.cam.tzoom = 2.1;
            // Centre between the drawer (if open) and the side panel.
            let li = if self.drawer.is_some() { self.width.min(860.0) } else { 0.0 };
            let ri = self.width.min(500.0);
            self.cam.tx = n.tx + ((ri - li) / 2.0) / 2.1;
            self.cam.ty = n.ty;
        }
        self.selection = Selection::Agent(addr.to_string());
        self.panel_dirty = true;
    }

    pub fn focus_channel(&mut self, a: &str, b: &str) {
        if let (Some(na), Some(nb)) = (self.nodes.get(a), self.nodes.get(b)) {
            let d = ((na.tx - nb.tx).powi(2) + (na.ty - nb.ty).powi(2)).sqrt();
            self.cam.tzoom = ((self.width * 0.5) / (d + 200.0)).clamp(0.3, 2.0);
            self.cam.tx = (na.tx + nb.tx) / 2.0 + (self.width * 0.17) / self.cam.tzoom;
            self.cam.ty = (na.ty + nb.ty) / 2.0;
        }
        self.selection = Selection::Channel(a.to_string(), b.to_string());
        self.panel_dirty = true;
    }

    pub fn matches_search(&self, a: &Agent) -> bool {
        if self.query.is_empty() {
            return true;
        }
        self.query.matches(a, self.insights.get(&a.addr), &self.name(&a.addr))
    }

    pub fn set_search(&mut self, s: &str) {
        self.search = s.to_string();
        self.query = Query::parse(s);
        self.drawer_dirty = true;
        self.hud_dirty = true;
    }
}

/// What sets a node's size: fresh tokens and cache writes. Cache reads are left
/// out so an agent re-reading a big cached context does not look like a heavy burner.
fn node_weight(t: &relay_map_core::Tokens) -> f64 {
    (t.fresh() + t.cache_write) as f64
}
