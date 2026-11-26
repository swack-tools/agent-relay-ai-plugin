//! relay-map: live map of agent-relay traffic.
//!
//! `relay-map serve`   collect here and over ssh, serve the web UI and WebSocket
//! `relay-map collect` print this computer's events as JSON lines (used over ssh)

mod broadcast;
mod collect;
mod config;
mod hub;
mod remote;
mod transcript;
mod web;

use anyhow::Result;
use clap::{Parser, Subcommand};
use relay_map_core::CollectorEvent;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "relay-map", version, about = "Live map of agent-relay traffic between Claude Code, Codex, Antigravity and pi sessions")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the hub: collect locally and from ssh hosts, serve the web UI.
    Serve {
        /// Config file (default: ./relay-map.toml if present).
        #[arg(long)]
        config: Option<PathBuf>,
        /// Address to listen on (overrides the config).
        #[arg(long)]
        listen: Option<String>,
        /// Extra ssh host to collect from (repeatable).
        #[arg(long = "ssh")]
        ssh: Vec<String>,
        #[arg(long)]
        lookback_hours: Option<f64>,
        /// Folder with the built front end.
        #[arg(long)]
        web_dir: Option<PathBuf>,
        /// Folder with collector binaries for remote platforms.
        #[arg(long)]
        agents_dir: Option<PathBuf>,
        /// Do not collect from this computer.
        #[arg(long)]
        no_local: bool,
    },
    /// Print this computer's agent events as JSON lines.
    Collect {
        #[arg(long, default_value_t = 72.0)]
        lookback_hours: f64,
        #[arg(long, default_value_t = 1500)]
        interval_ms: u64,
        /// Scan once and exit.
        #[arg(long)]
        once: bool,
    },
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Collect { lookback_hours, interval_ms, once } => {
            let stdout = std::io::stdout();
            let mut out = std::io::BufWriter::new(stdout.lock());
            let mut emit = |e: CollectorEvent| -> bool {
                let ok = serde_json::to_writer(&mut out, &e).is_ok() && out.write_all(b"\n").is_ok();
                // Flush per event once live so the hub sees traffic promptly.
                ok && out.flush().is_ok()
            };
            let mut c = collect::Collector::new(lookback_hours);
            if once {
                let started = std::time::Instant::now();
                emit(CollectorEvent::Hello { host: c.host.clone(), version: env!("CARGO_PKG_VERSION").into(), roots: c.roots() });
                let agents = c.scan(&mut |e| {
                    emit(e);
                });
                emit(CollectorEvent::Ready { agents, messages: 0, millis: started.elapsed().as_millis() as u64 });
                return Ok(());
            }
            c.run(Duration::from_millis(interval_ms), &mut emit);
            Ok(())
        }
        Cmd::Serve { config, listen, ssh, lookback_hours, web_dir, agents_dir, no_local } => {
            let (mut cfg, cfg_path) = config::Config::load(config.as_deref())?;
            if let Some(l) = listen {
                cfg.listen = l;
            }
            if let Some(h) = lookback_hours {
                cfg.lookback_hours = h;
            }
            if no_local {
                cfg.local = false;
            }
            for s in ssh {
                if !cfg.hosts.iter().any(|h| h.name == s) {
                    cfg.hosts.push(config::HostCfg { name: s, ssh: None });
                }
            }
            let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
            rt.block_on(serve(cfg, cfg_path, web_dir, agents_dir))
        }
    }
}

fn default_dir(name: &str) -> PathBuf {
    // Next to the binary's workspace (web-ui/), or the current directory.
    let exe = std::env::current_exe().ok();
    let mut d = exe.as_deref().and_then(|p| p.parent());
    while let Some(dir) = d {
        if dir.join(name).is_dir() {
            return dir.join(name);
        }
        d = dir.parent();
    }
    PathBuf::from(name)
}

async fn serve(cfg: config::Config, cfg_path: Option<PathBuf>, web_dir: Option<PathBuf>, agents_dir: Option<PathBuf>) -> Result<()> {
    let self_host = collect::self_host();
    let web_dir = web_dir.or(cfg.web_dir.clone()).unwrap_or_else(|| default_dir("dist"));
    let agents_dir = agents_dir.or(cfg.agents_dir.clone()).unwrap_or_else(|| default_dir("bin"));
    let relay_py = broadcast::find_relay_py(cfg.relay_py.as_deref());
    let listen = cfg.listen.clone();
    let lookback = cfg.lookback_hours;
    let hosts = cfg.hosts.clone();
    let local = cfg.local;
    eprintln!("relay-map: config {}", cfg_path.map(|p| p.display().to_string()).unwrap_or_else(|| "built-in defaults".into()));
    eprintln!("relay-map: web UI from {}", web_dir.display());
    eprintln!("relay-map: relay.py {}", relay_py.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "NOT FOUND (broadcast disabled)".into()));
    let hub = hub::Hub::new(cfg, self_host.clone());

    if local {
        let hub2 = hub.clone();
        let name = self_host.clone();
        hub.lock().unwrap().set_host(&name, "local", "backfilling", &format!("reading the last {lookback} h of transcripts"));
        std::thread::spawn(move || {
            let c = collect::Collector::new(lookback);
            let roots = c.roots().join(", ");
            let started = std::time::Instant::now();
            let mut ready = false;
            let mut batch = Vec::new();
            c.run(Duration::from_millis(1500), &mut |e| {
                if let CollectorEvent::Ready { agents, messages, millis } = &e {
                    ready = true;
                    let mut h = hub2.lock().unwrap();
                    for b in batch.drain(..) {
                        h.ingest(b);
                    }
                    let d = format!("live ({agents} agents, {messages} messages, backfill {:.1} s) from {roots}", *millis as f64 / 1000.0);
                    h.set_host(&name, "local", "live", &d);
                    return true;
                }
                if ready {
                    hub2.lock().unwrap().ingest(e);
                } else {
                    batch.push(e);
                    if batch.len() > 2000 || started.elapsed() > Duration::from_secs(600) {
                        let mut h = hub2.lock().unwrap();
                        for b in batch.drain(..) {
                            h.ingest(b);
                        }
                    }
                }
                true
            });
        });
    }
    for h in hosts {
        tokio::spawn(remote::run(h, hub.clone(), agents_dir.clone(), lookback));
    }
    let app = web::router(web::AppState { hub, relay_py }, web_dir);
    let listener = tokio::net::TcpListener::bind(&listen).await?;
    eprintln!("relay-map: open http://{listen}/");
    axum::serve(listener, app).with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await?;
    Ok(())
}
