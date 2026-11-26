//! Remote collectors: copy the matching `relay-map` binary to a computer over
//! ssh (once per build) and stream `relay-map collect` from it.

use crate::config::HostCfg;
use crate::hub::Shared;
use anyhow::{bail, Context, Result};
use relay_map_core::CollectorEvent;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

fn ssh_base(target: &str) -> Command {
    let mut c = Command::new("ssh");
    c.args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=10", "-o", "ServerAliveInterval=15", "-o", "ServerAliveCountMax=3", target]);
    c.kill_on_drop(true);
    c
}

async fn ssh_output(target: &str, cmd: &str) -> Result<String> {
    let out = ssh_base(target).arg(cmd).stdin(Stdio::null()).output().await?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Collector binary for `uname -sm` output.
pub fn binary_for(agents_dir: &Path, uname: &str) -> Option<PathBuf> {
    let mut parts = uname.split_whitespace();
    let os = match parts.next()? {
        "Darwin" => "macos",
        "Linux" => "linux",
        _ => return None,
    };
    let arch = match parts.next()? {
        "arm64" | "aarch64" => "aarch64",
        "x86_64" | "amd64" => "x86_64",
        _ => return None,
    };
    let p = agents_dir.join(format!("relay-map-{os}-{arch}"));
    if p.is_file() {
        return Some(p);
    }
    // The hub's own binary works on a computer of the same platform.
    let me = std::env::current_exe().ok()?;
    let same = (cfg!(target_os = "macos") && os == "macos" || cfg!(target_os = "linux") && os == "linux")
        && (cfg!(target_arch = "aarch64") && arch == "aarch64" || cfg!(target_arch = "x86_64") && arch == "x86_64");
    same.then_some(me)
}

async fn deploy(target: &str, bin: &Path) -> Result<String> {
    let bytes = tokio::fs::read(bin).await.with_context(|| format!("reading {}", bin.display()))?;
    let remote = format!(".cache/relay-map/relay-map-{:016x}", fnv(&bytes));
    let check = format!("test -x {remote} && echo present || echo missing");
    if ssh_output(target, &check).await? == "present" {
        return Ok(remote);
    }
    let script = format!("mkdir -p .cache/relay-map && cat > {remote}.tmp && chmod +x {remote}.tmp && mv {remote}.tmp {remote}");
    let mut child = ssh_base(target).arg(script).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn()?;
    child.stdin.take().unwrap().write_all(&bytes).await?;
    let out = child.wait_with_output().await?;
    if !out.status.success() {
        bail!("copying the collector failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(remote)
}

async fn session(host: &HostCfg, hub: &Shared, agents_dir: &Path, lookback: f64) -> Result<()> {
    let target = host.ssh_target();
    let set = |state: &str, detail: &str| hub.lock().unwrap().set_host(&host.name, "ssh", state, detail);
    set("connecting", &format!("ssh {target}"));
    let uname = ssh_output(target, "uname -sm").await.context("ssh failed")?;
    let Some(bin) = binary_for(agents_dir, &uname) else {
        bail!("no collector binary for {uname}; run build.sh to cross-compile one into {}", agents_dir.display());
    };
    let remote = deploy(target, &bin).await?;
    set("backfilling", &format!("{uname}; reading the last {lookback} h of transcripts"));
    let mut child = ssh_base(target)
        .arg(format!("{remote} collect --lookback-hours {lookback}"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let mut lines = BufReader::new(stdout).lines();
    while let Some(line) = lines.next_line().await? {
        let Ok(ev) = serde_json::from_str::<CollectorEvent>(&line) else { continue };
        let mut h = hub.lock().unwrap();
        match &ev {
            CollectorEvent::Ready { agents, messages, millis } => {
                let d = format!("{uname}; live ({agents} agents, {messages} messages, backfill {:.1} s)", *millis as f64 / 1000.0);
                h.set_host(&host.name, "ssh", "live", &d);
            }
            CollectorEvent::Hello { host: reported, .. } if reported != &host.name => {
                let d = format!("collector reports itself as {reported}; addresses use that name");
                h.set_host(&host.name, "ssh", "backfilling", &d);
            }
            _ => {}
        }
        h.ingest(ev);
    }
    let mut err = String::new();
    let _ = stderr.read_to_string(&mut err).await;
    let status = child.wait().await?;
    bail!("collector exited ({status}): {}", err.trim().lines().last().unwrap_or(""));
}

pub async fn run(host: HostCfg, hub: Shared, agents_dir: PathBuf, lookback: f64) {
    let mut backoff = 5;
    loop {
        if let Err(e) = session(&host, &hub, &agents_dir, lookback).await {
            hub.lock().unwrap().set_host(&host.name, "ssh", "error", &format!("{e:#}; retrying in {backoff} s"));
        }
        tokio::time::sleep(Duration::from_secs(backoff)).await;
        backoff = (backoff * 2).min(120);
    }
}
