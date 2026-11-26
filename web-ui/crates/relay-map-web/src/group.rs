//! Nested grouping: agents are grouped by an ordered list of levels (computer,
//! project, folder, task, …) and drawn as circles packed inside circles.

use crate::util::*;
use relay_map_core::*;
use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Host,
    Pool,
    Project,
    Directory,
    Kind,
    Task,
    Status,
}

pub const ALL_LEVELS: [Level; 7] = [Level::Host, Level::Project, Level::Directory, Level::Task, Level::Pool, Level::Kind, Level::Status];

impl Level {
    pub fn id(self) -> &'static str {
        match self {
            Level::Host => "host",
            Level::Pool => "pool",
            Level::Project => "project",
            Level::Directory => "directory",
            Level::Kind => "kind",
            Level::Task => "task",
            Level::Status => "status",
        }
    }
    pub fn from_id(s: &str) -> Option<Level> {
        ALL_LEVELS.iter().copied().find(|l| l.id() == s)
    }
    pub fn title(self) -> &'static str {
        match self {
            Level::Host => "Computer",
            Level::Pool => "Pool (from relay-map.toml)",
            Level::Project => "Project (repository)",
            Level::Directory => "Folder",
            Level::Kind => "Agent type",
            Level::Task => "Current task",
            Level::Status => "Status",
        }
    }
}

/// The group an agent falls in at one level: (key, label, colour).
pub fn key(level: Level, a: &Agent, pools: &[Pool], now: i64) -> (String, String, Option<Rgb>) {
    match level {
        Level::Host => (a.host.clone(), a.host.clone(), Some(parse_hex(host_color(&a.host)))),
        Level::Pool => match a.pool.as_deref().and_then(|n| pools.iter().find(|p| p.name == n && !p.auto)) {
            Some(p) => (p.name.clone(), p.name.clone(), Some(parse_hex(&p.color))),
            None => ("~none".into(), "no pool".into(), None),
        },
        Level::Project => {
            let p = if a.ghost { "seen in messages".to_string() } else { project_of(a.cwd.as_deref().unwrap_or("")) };
            (p.clone(), p, None)
        }
        Level::Directory => {
            let c = a.cwd.clone().unwrap_or_default();
            let label = if c.is_empty() { "(no folder)".to_string() } else { short_path(&c, 2) };
            (c, label, None)
        }
        Level::Kind => {
            let k = a.kind.map(|k| k.as_str()).unwrap_or("other");
            let label = if k == "codex" { "openai codex" } else { k };
            (k.to_string(), label.to_string(), Some(crate::app::kind_color(a.kind)))
        }
        Level::Task => {
            let t = a.title.as_deref().or(a.focus.as_deref()).or(a.last_text.as_deref()).unwrap_or("");
            if t.is_empty() {
                ("~".into(), "(no task recorded)".into(), None)
            } else {
                let words: Vec<String> = t.split_whitespace().take(5).map(|w| w.to_lowercase()).collect();
                let k = words.join(" ");
                (k, truncate(&t.split_whitespace().take(5).collect::<Vec<_>>().join(" "), 40), None)
            }
        }
        Level::Status => {
            if a.ghost {
                ("3".into(), "seen in messages".into(), None)
            } else if a.alive == Some(true) || now - a.last_active_ms < 5 * 60 * 1000 {
                ("0".into(), "active now".into(), Some(parse_hex("#46f5a0")))
            } else if a.alive == Some(false) {
                ("2".into(), "ended".into(), Some(parse_hex("#8892b0")))
            } else {
                ("1".into(), "idle".into(), Some(parse_hex("#ffd166")))
            }
        }
    }
}

pub struct GNode {
    pub label: String,
    pub color: Option<Rgb>,
    pub children: Vec<GNode>,
    pub agents: Vec<String>,
    pub r: f64,
    /// Centre relative to the parent's centre.
    pub x: f64,
    pub y: f64,
    pub level: Option<Level>,
}

/// Build the tree of groups, collapsing a group that has only one sub-group.
pub fn build(members: Vec<String>, levels: &[Level], keyf: &dyn Fn(Level, &str) -> (String, String, Option<Rgb>), label: String, color: Option<Rgb>, level: Option<Level>) -> GNode {
    let Some((&first, rest)) = levels.split_first() else {
        return GNode { label, color, children: vec![], agents: members, r: 0.0, x: 0.0, y: 0.0, level };
    };
    let mut groups: BTreeMap<String, (String, Option<Rgb>, Vec<String>)> = BTreeMap::new();
    for m in members {
        let (k, l, c) = keyf(first, &m);
        groups.entry(k).or_insert_with(|| (l, c, vec![])).2.push(m);
    }
    if groups.len() == 1 && level.is_some() {
        // A single sub-group adds nothing: fold its name into this one.
        let (_, (l, c, ms)) = groups.into_iter().next().unwrap();
        // Skip names that repeat the parent ("web-app" / "…/git/web-app") or say nothing.
        let merged = if l == label || l.starts_with('(') || l.ends_with(&format!("/{label}")) || label.is_empty() && l.is_empty() {
            label
        } else {
            format!("{label} / {l}")
        };
        return build(ms, rest, keyf, merged, color.or(c), level);
    }
    let children = groups.into_values().map(|(l, c, ms)| build(ms, rest, keyf, l, c, Some(first))).collect();
    GNode { label, color, children, agents: vec![], r: 0.0, x: 0.0, y: 0.0, level }
}

/// Size every group and pack its children (circle packing, biggest first).
pub fn pack(n: &mut GNode, agent_r: &dyn Fn(&str) -> f64) {
    if n.children.is_empty() {
        let k = n.agents.len() as f64;
        let biggest = n.agents.iter().map(|a| agent_r(a)).fold(8.0, f64::max);
        n.r = if n.agents.len() == 1 { biggest + 12.0 } else { (biggest + 26.0).max(20.0 + 17.0 * k.sqrt()) };
        return;
    }
    for c in n.children.iter_mut() {
        pack(c, agent_r);
    }
    let gap = 14.0;
    let mut order: Vec<usize> = (0..n.children.len()).collect();
    order.sort_by(|&a, &b| n.children[b].r.partial_cmp(&n.children[a].r).unwrap());
    let mut placed: Vec<(f64, f64, f64)> = Vec::new();
    for &i in &order {
        let r = n.children[i].r;
        let (x, y) = if placed.is_empty() { (0.0, 0.0) } else { find_spot(&placed, r, gap) };
        n.children[i].x = x;
        n.children[i].y = y;
        placed.push((x, y, r));
    }
    // Recentre on the bounding circle.
    let (cx, cy) = {
        let (mut minx, mut maxx, mut miny, mut maxy) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for &(x, y, r) in &placed {
            minx = minx.min(x - r);
            maxx = maxx.max(x + r);
            miny = miny.min(y - r);
            maxy = maxy.max(y + r);
        }
        ((minx + maxx) / 2.0, (miny + maxy) / 2.0)
    };
    let mut rad: f64 = 0.0;
    for c in n.children.iter_mut() {
        c.x -= cx;
        c.y -= cy;
        rad = rad.max((c.x * c.x + c.y * c.y).sqrt() + c.r);
    }
    n.r = rad + 22.0;
}

/// First position (walking outwards) where a circle of radius r fits.
pub fn find_spot(placed: &[(f64, f64, f64)], r: f64, gap: f64) -> (f64, f64) {
    let mut d = placed[0].2 + r + gap;
    loop {
        let steps = ((std::f64::consts::TAU * d) / (r * 0.6 + 8.0)).ceil().max(12.0) as usize;
        let off = d * 0.37;
        for s in 0..steps {
            let th = off + s as f64 / steps as f64 * std::f64::consts::TAU;
            let (x, y) = (d * th.cos(), d * th.sin());
            if placed.iter().all(|&(px, py, pr)| ((x - px).powi(2) + (y - py).powi(2)).sqrt() >= pr + r + gap) {
                return (x, y);
            }
        }
        d += (r * 0.25).max(6.0);
    }
}
