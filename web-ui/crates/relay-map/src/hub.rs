//! The hub merges collector streams from every computer, resolves message
//! endpoints to agents, assigns pools and fans updates out to browsers.

use crate::config::Config;
use relay_map_core::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

pub const MAX_MESSAGES: usize = 6000;
/// How close a `relay send` stub and the delivered message must be to merge.
const STUB_WINDOW_MS: i64 = 5 * 60 * 1000;

pub struct Hub {
    pub cfg: Config,
    pub self_host: String,
    pub agents: HashMap<String, Agent>,
    pub messages: Vec<RelayMessage>,
    pub hosts: HashMap<String, HostStatus>,
    /// Computer + project pools, by name.
    pub auto_pools: std::collections::BTreeMap<String, Pool>,
    pub tx: broadcast::Sender<ServerMsg>,
}

pub type Shared = Arc<Mutex<Hub>>;

pub fn operator_addr(host: &str) -> String {
    format!("operator:relay-map@{host}")
}

impl Hub {
    pub fn new(cfg: Config, self_host: String) -> Shared {
        let (tx, _) = broadcast::channel(4096);
        let mut hub = Hub { cfg, self_host: self_host.clone(), agents: HashMap::new(), messages: Vec::new(), hosts: HashMap::new(), auto_pools: Default::default(), tx };
        let op = Agent {
            addr: operator_addr(&self_host),
            kind: Some(AgentKind::Operator),
            id: "relay-map".into(),
            host: self_host,
            label: Some("You (Relay Map)".into()),
            title: Some("Broadcasts from this dashboard".into()),
            alive: Some(true),
            ..Default::default()
        };
        hub.agents.insert(op.addr.clone(), op);
        Arc::new(Mutex::new(hub))
    }

    pub fn snapshot(&self) -> ServerMsg {
        let mut hosts: Vec<HostStatus> = self.hosts.values().cloned().collect();
        hosts.sort_by(|a, b| a.name.cmp(&b.name));
        ServerMsg::Snapshot {
            now_ms: crate::collect::now_ms(),
            self_host: self.self_host.clone(),
            agents: self.agents.values().cloned().collect(),
            messages: self.messages.clone(),
            pools: self.all_pools(),
            hosts,
        }
    }

    fn send(&self, m: ServerMsg) {
        let _ = self.tx.send(m);
    }

    pub fn set_host(&mut self, name: &str, via: &str, state: &str, detail: &str) {
        let agents = self.agents.values().filter(|a| !a.ghost && a.host == name).count();
        let h = HostStatus { name: name.into(), via: via.into(), state: state.into(), detail: detail.into(), agents };
        self.hosts.insert(name.to_string(), h.clone());
        self.send(ServerMsg::Host(h));
    }

    pub fn ingest(&mut self, ev: CollectorEvent) {
        match ev {
            CollectorEvent::Agent(a) => self.upsert_agent(*a),
            CollectorEvent::Message(m) => self.add_message(m),
            CollectorEvent::Hello { .. } | CollectorEvent::Ready { .. } | CollectorEvent::Note { .. } => {}
        }
    }

    pub fn all_pools(&self) -> Vec<Pool> {
        let mut p = self.cfg.pools();
        p.extend(self.auto_pools.values().cloned());
        p
    }

    /// Configured pool, else a pool for this computer and project.
    fn assign_pool(&mut self, a: &Agent) -> Option<String> {
        if let Some(p) = self.cfg.pool_for(a) {
            return Some(p);
        }
        // Swarm members belong with the orchestrator that spawned them.
        if let Some(parent) = a.parent.as_ref().and_then(|p| self.agents.get(p)) {
            if parent.pool.is_some() {
                return parent.pool.clone();
            }
        }
        if !self.cfg.auto_pools || a.kind == Some(AgentKind::Operator) || a.host.is_empty() {
            return None;
        }
        let project = if a.ghost { "seen in messages".to_string() } else { project_of(a.cwd.as_deref().unwrap_or("")) };
        let name = format!("{} · {}", a.host, project);
        if !self.auto_pools.contains_key(&name) {
            self.auto_pools.insert(
                name.clone(),
                Pool {
                    name: name.clone(),
                    color: host_color(&a.host).to_string(),
                    description: format!("Agents in {project} on {}", a.host),
                    isolated: false,
                    auto: true,
                    host: Some(a.host.clone()),
                    project: Some(project),
                },
            );
            let pools = self.all_pools();
            self.send(ServerMsg::Pools { pools });
        }
        Some(name)
    }

    pub fn upsert_agent(&mut self, mut a: Agent) {
        a.pool = self.assign_pool(&a);
        let is_new = !self.agents.contains_key(&a.addr);
        let addr = a.addr.clone();
        let pool = a.pool.clone();
        self.agents.insert(a.addr.clone(), a.clone());
        self.send(ServerMsg::Agent(Box::new(a)));
        // Children that arrived before their orchestrator follow it now.
        let kids: Vec<String> = self
            .agents
            .values()
            .filter(|c| c.parent.as_deref() == Some(addr.as_str()) && c.pool != pool && self.cfg.pool_for(c).is_none())
            .map(|c| c.addr.clone())
            .collect();
        for k in kids {
            if let Some(c) = self.agents.get_mut(&k) {
                c.pool = pool.clone();
                let c = c.clone();
                self.send(ServerMsg::Agent(Box::new(c)));
            }
        }
        if is_new {
            self.reresolve();
        }
    }

    /// Resolve an address as written to an agent address, if a known agent matches.
    fn resolve(&self, text: &str, default_host: &str) -> Option<String> {
        let addr = Addr::parse(text)?.with_default_host(default_host);
        let host = addr.host.clone().unwrap_or_default();
        let r = addr.reference.as_str();
        let full = addr.render();
        if self.agents.get(&full).is_some_and(|a| !a.ghost) {
            return Some(full);
        }
        let cands: Vec<&Agent> = self
            .agents
            .values()
            .filter(|a| !a.ghost && a.host == host && a.kind.map(|k| k.as_str()) == Some(addr.kind.as_str()))
            .collect();
        let pick = |f: &dyn Fn(&Agent) -> bool| -> Option<String> {
            let hits: Vec<&&Agent> = cands.iter().filter(|a| f(a)).collect();
            if hits.len() == 1 {
                Some(hits[0].addr.clone())
            } else {
                // Several matches: take the most recently active one.
                hits.iter().max_by_key(|a| a.last_active_ms).map(|a| a.addr.clone())
            }
        };
        pick(&|a| a.id == r)
            .or_else(|| if r.len() >= 6 { pick(&|a| a.id.starts_with(r)) } else { None })
            .or_else(|| pick(&|a| a.label.as_deref() == Some(r)))
            .or_else(|| pick(&|a| a.cwd.as_deref().is_some_and(|c| c.trim_end_matches('/').rsplit('/').next() == Some(r))))
    }

    /// Make sure an address that matches no agent still has a (ghost) node.
    fn ensure_ghost(&mut self, text: &str, default_host: &str) -> String {
        let Some(addr) = Addr::parse(text).map(|a| a.with_default_host(default_host)) else {
            return text.to_string();
        };
        let key = addr.render();
        if !self.agents.contains_key(&key) {
            let kind = AgentKind::parse(&addr.kind);
            let g = Agent {
                addr: key.clone(),
                kind,
                id: addr.reference.clone(),
                host: addr.host.clone().unwrap_or_default(),
                label: Some(if addr.reference.len() > 12 { addr.reference[..8].to_string() } else { addr.reference.clone() }),
                title: Some("Seen only in relay messages (its computer is not monitored, or the session has ended)".into()),
                ghost: true,
                ..Default::default()
            };
            let mut g = g;
            g.pool = self.assign_pool(&g);
            self.agents.insert(key.clone(), g.clone());
            self.send(ServerMsg::Agent(Box::new(g)));
        }
        key
    }

    fn endpoints(&mut self, m: &mut RelayMessage) {
        let from_host = Addr::parse(&m.from).and_then(|a| a.host).unwrap_or_else(|| self.self_host.clone());
        m.from_agent = self.resolve(&m.from, &from_host).or_else(|| Some(self.ensure_ghost(&m.from, &from_host)));
        // A missing host on the recipient means "the sender's computer".
        m.to_agent = self.resolve(&m.to, &from_host).or_else(|| Some(self.ensure_ghost(&m.to, &from_host)));
    }

    fn reresolve(&mut self) {
        let mut changed = Vec::new();
        for i in 0..self.messages.len() {
            let ghostly = |x: &Option<String>, agents: &HashMap<String, Agent>| x.as_ref().and_then(|a| agents.get(a)).is_none_or(|a| a.ghost);
            let m = &self.messages[i];
            if !ghostly(&m.from_agent, &self.agents) && !ghostly(&m.to_agent, &self.agents) {
                continue;
            }
            let mut m = m.clone();
            let before = (m.from_agent.clone(), m.to_agent.clone());
            let from_host = Addr::parse(&m.from).and_then(|a| a.host).unwrap_or_else(|| self.self_host.clone());
            if let Some(f) = self.resolve(&m.from, &from_host) {
                m.from_agent = Some(f);
            }
            if let Some(t) = self.resolve(&m.to, &from_host) {
                m.to_agent = Some(t);
            }
            if (m.from_agent.clone(), m.to_agent.clone()) != before {
                self.messages[i] = m.clone();
                changed.push(m);
            }
        }
        for m in changed {
            self.send(ServerMsg::Message(m));
        }
    }

    pub fn add_message(&mut self, mut m: RelayMessage) {
        if m.id.is_empty() {
            return;
        }
        self.endpoints(&mut m);
        // A real id replacing a stub from the same collector.
        if let Some(stub) = m.replaces.clone() {
            self.messages.retain(|x| x.id != stub);
        }
        // Delivered copy of a message whose `relay send` stub we already have.
        if m.source != MsgSource::Sent && !m.id.starts_with("send:") {
            if let Some(pos) = self.messages.iter().position(|x| {
                x.id.starts_with("send:")
                    && x.from_agent == m.from_agent
                    && x.to_agent == m.to_agent
                    && (x.sent_ms - m.sent_ms).abs() < STUB_WINDOW_MS
            }) {
                let stub = self.messages.remove(pos);
                m.replaces = Some(stub.id);
            }
        }
        if let Some(existing) = self.messages.iter_mut().find(|x| x.id == m.id) {
            // Same message seen from several places: keep the richest copy.
            let better_body = m.body.len() > existing.body.len() && m.source != MsgSource::Sent;
            if better_body || existing.source == MsgSource::Sent && m.source != MsgSource::Sent {
                existing.body = m.body.clone();
                existing.bytes = existing.bytes.max(m.bytes);
            }
            if existing.subject.is_none() {
                existing.subject = m.subject.clone();
            }
            // The inbox and the transcripts know the exact recipient better than a stub.
            if m.source == MsgSource::Inbox || m.source == MsgSource::Received {
                existing.to_agent = m.to_agent.clone();
                existing.from_agent = m.from_agent.clone();
                existing.source = m.source;
            }
            let e = existing.clone();
            self.send(ServerMsg::Message(e));
            return;
        }
        let pos = self.messages.partition_point(|x| x.sent_ms <= m.sent_ms);
        self.messages.insert(pos, m.clone());
        if self.messages.len() > MAX_MESSAGES {
            self.messages.remove(0);
        }
        self.send(ServerMsg::Message(m));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(kind: AgentKind, id: &str, host: &str, cwd: &str) -> Agent {
        Agent { addr: agent_addr(kind, id, host), kind: Some(kind), id: id.into(), host: host.into(), cwd: Some(cwd.into()), last_active_ms: 1, ..Default::default() }
    }

    fn msg(id: &str, from: &str, to: &str, source: MsgSource, at: i64) -> RelayMessage {
        RelayMessage { id: id.into(), from: from.into(), to: to.into(), subject: None, body: "hi".into(), bytes: 2, sent_ms: at, source, replaces: None, from_agent: None, to_agent: None }
    }

    #[test]
    fn resolves_labels_and_merges_stubs() {
        let mut cfg = Config::default();
        cfg.pools.push(crate::config::PoolCfg {
            name: "Backend".into(),
            color: "#ff9d3c".into(),
            description: String::new(),
            isolated: true,
            patterns: vec!["api-server".into(), "worker".into()],
            match_title: vec![],
            kinds: vec![],
            hosts: vec![],
            agents: vec![],
        });
        let hub = Hub::new(cfg, "here".into());
        let mut h = hub.lock().unwrap();
        h.upsert_agent(agent(AgentKind::Claude, "aaaaaaaa-1111", "here", "/x/api-server"));
        h.upsert_agent(agent(AgentKind::Claude, "bbbbbbbb-2222", "far", "/y/worker"));
        assert_eq!(h.agents["claude:aaaaaaaa-1111@here"].pool.as_deref(), Some("Backend"));
        // A `relay send --to claude:worker@far` stub resolves by folder name.
        h.add_message(msg("send:x:1", "claude:aaaaaaaa-1111@here", "claude:worker@far", MsgSource::Sent, 1000));
        assert_eq!(h.messages[0].to_agent.as_deref(), Some("claude:bbbbbbbb-2222@far"));
        // The delivered envelope replaces the stub instead of adding a second message.
        h.add_message(msg("20261123T000000-deadbeef", "claude:aaaaaaaa-1111@here", "claude:bbbbbbbb-2222@far", MsgSource::Inbox, 2000));
        assert_eq!(h.messages.len(), 1);
        assert_eq!(h.messages[0].id, "20261123T000000-deadbeef");
        // The same message seen again in the recipient's transcript is merged.
        h.add_message(msg("20261123T000000-deadbeef", "claude:aaaaaaaa-1111@here", "claude:bbbbbbbb-2222@far", MsgSource::Received, 2000));
        assert_eq!(h.messages.len(), 1);
        // Unknown senders become ghost nodes.
        h.add_message(msg("20261123T000001-cafef00d", "codex:nobody@elsewhere", "claude:bbbbbbbb-2222@far", MsgSource::Inbox, 3000));
        assert!(h.agents["codex:nobody@elsewhere"].ghost);
    }
}
