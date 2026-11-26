//! `relay-map.toml`: hosts to collect from and communication pools.

use anyhow::{Context, Result};
use relay_map_core::{Agent, Pool};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_listen")]
    pub listen: String,
    #[serde(default = "default_lookback")]
    pub lookback_hours: f64,
    /// Path to agent-relay's `relay.py`; found automatically when unset.
    pub relay_py: Option<PathBuf>,
    /// Folder with the built front end (`index.html`, wasm).
    pub web_dir: Option<PathBuf>,
    /// Folder with collector binaries for remote computers (`relay-map-<os>-<arch>`).
    pub agents_dir: Option<PathBuf>,
    #[serde(default = "yes")]
    pub local: bool,
    /// Group agents that match no pool by computer and project.
    #[serde(default = "yes")]
    pub auto_pools: bool,
    #[serde(default)]
    pub hosts: Vec<HostCfg>,
    /// VictoriaLogs base URL (`http://host:9428`), the last place the agent
    /// panel looks for a session log, by `session_id` field.
    #[serde(default)]
    pub victorialogs_url: Option<String>,
    #[serde(default)]
    pub pools: Vec<PoolCfg>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostCfg {
    /// Display name; also the ssh alias unless `ssh` is set.
    pub name: String,
    pub ssh: Option<String>,
}

impl HostCfg {
    pub fn ssh_target(&self) -> &str {
        self.ssh.as_deref().unwrap_or(&self.name)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PoolCfg {
    pub name: String,
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "yes")]
    pub isolated: bool,
    /// Case-insensitive substrings matched against an agent's folder and label.
    #[serde(default, rename = "match")]
    pub patterns: Vec<String>,
    /// Also match these substrings against the session title.
    #[serde(default)]
    pub match_title: Vec<String>,
    /// Restrict to these agent kinds (`claude`, `codex`, `antigravity`, `pi`).
    #[serde(default)]
    pub kinds: Vec<String>,
    /// Restrict to these hosts.
    #[serde(default)]
    pub hosts: Vec<String>,
    /// Specific agents, by address or session id (a prefix of 6+ characters is enough).
    /// These join the pool whatever their folder.
    #[serde(default)]
    pub agents: Vec<String>,
}

fn default_listen() -> String {
    "127.0.0.1:7878".into()
}
fn default_lookback() -> f64 {
    72.0
}
fn default_color() -> String {
    "#9aa7ff".into()
}
fn yes() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        toml::from_str(DEFAULT_TOML).expect("built-in config parses")
    }
}

/// Used when no config file exists: auto_pools groups agents by computer and project.
pub const DEFAULT_TOML: &str = "";

impl Config {
    pub fn load(path: Option<&Path>) -> Result<(Config, Option<PathBuf>)> {
        let candidates: Vec<PathBuf> = match path {
            Some(p) => vec![p.to_path_buf()],
            None => vec![PathBuf::from("relay-map.toml")],
        };
        for p in candidates {
            if p.is_file() {
                let text = std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?;
                let mut cfg: Config = toml::from_str(&text).with_context(|| format!("parsing {}", p.display()))?;
                if cfg.pools.is_empty() {
                    cfg.pools = Config::default().pools;
                }
                return Ok((cfg, Some(p)));
            } else if path.is_some() {
                anyhow::bail!("config file {} not found", p.display());
            }
        }
        Ok((Config::default(), None))
    }

    pub fn pools(&self) -> Vec<Pool> {
        self.pools
            .iter()
            .map(|p| Pool {
                name: p.name.clone(),
                color: p.color.clone(),
                description: p.description.clone(),
                isolated: p.isolated,
                auto: false,
                host: None,
                project: None,
            })
            .collect()
    }

    pub fn pool_for(&self, a: &Agent) -> Option<String> {
        let hay = format!(
            "{} {}",
            a.cwd.as_deref().unwrap_or("").to_lowercase(),
            a.label.as_deref().unwrap_or("").to_lowercase()
        );
        let title = a.title.as_deref().unwrap_or("").to_lowercase();
        let kind = a.kind.map(|k| k.as_str()).unwrap_or("");
        for p in &self.pools {
            if p.agents.iter().any(|x| x == &a.addr || (x.len() >= 6 && a.id.starts_with(x.as_str()))) {
                return Some(p.name.clone());
            }
        }
        for p in &self.pools {
            if !p.kinds.is_empty() && !p.kinds.iter().any(|k| k == kind) {
                continue;
            }
            if !p.hosts.is_empty() && !p.hosts.iter().any(|h| h == &a.host) {
                continue;
            }
            let hit = p.patterns.iter().any(|m| hay.contains(&m.to_lowercase()))
                || p.match_title.iter().any(|m| title.contains(&m.to_lowercase()));
            if hit {
                return Some(p.name.clone());
            }
        }
        None
    }
}
