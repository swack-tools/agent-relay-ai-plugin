//! Operator views: the attention / agents / messages drawer, the watch list,
//! the traffic timeline, and the per-agent activity strip.

use crate::app::*;
use crate::insight::{self, Health};
use crate::util::*;
use relay_map_core::*;
use std::collections::HashSet;

fn health_pill(h: Health) -> String {
    format!("<span class=\"hp\" style=\"color:{0};border-color:{0}\">{1}</span>", rgba(h.color(), 1.0), h.label())
}

fn kind_short(k: Option<AgentKind>) -> String {
    let c = kind_color(k);
    let n = match k {
        Some(AgentKind::Codex) => "codex",
        Some(k) => k.as_str(),
        None => "agent",
    };
    format!("<span style=\"color:{}\">{}</span>", rgba(c, 1.0), n)
}

pub fn render_drawer(app: &App) {
    let Some(tab) = app.drawer else {
        show("drawer", false);
        return;
    };
    show("drawer", true);
    let n0 = app.alerts.iter().filter(|a| a.sev == 0).count();
    let shown = app.ordered_matches().len();
    for (id, t, label) in [
        ("tab-attention", Tab::Attention, format!("⚠ Attention <em>{}</em>", app.alerts.len())),
        ("tab-agents", Tab::Agents, format!("Agents <em>{shown}</em>")),
        ("tab-messages", Tab::Messages, "Messages".to_string()),
        ("tab-spend", Tab::Spend, "$ Spend".to_string()),
    ] {
        if let Some(e) = el(id) {
            e.set_inner_html(&label);
            let _ = if t == tab { e.class_list().add_1("on") } else { e.class_list().remove_1("on") };
            let _ = if id == "tab-attention" && n0 > 0 { e.class_list().add_1("hot") } else { e.class_list().remove_1("hot") };
        }
    }
    let body = match tab {
        Tab::Attention => attention(app),
        Tab::Agents => agents_table(app),
        Tab::Messages => messages(app),
        Tab::Spend => spend(app),
    };
    let scroll = el("drawer-body").map_or(0, |b| b.scroll_top());
    set_html("drawer-body", &body);
    if let Some(b) = el("drawer-body") {
        b.set_scroll_top(scroll);
    }
}

fn notify_toggle(app: &App) -> String {
    let denied = crate::notify::permission() == web_sys::NotificationPermission::Denied;
    format!(
        "<div class=\"notify\"><button data-action=\"notify\" class=\"{}\">🔔 Desktop alerts: <b>{}</b></button> <span class=\"muted small\">{}</span></div>",
        if app.notify { "on" } else { "" },
        if app.notify { "on" } else { "off" },
        if denied {
            "Notifications are blocked for this page in the browser's site settings."
        } else {
            "A notification when an agent gets stuck, starts looping, or starts waiting on you. Click it to jump to the agent."
        }
    )
}

fn attention(app: &App) -> String {
    let toggle = notify_toggle(app);
    if app.alerts.is_empty() {
        return toggle +  "<p class=\"calm\">✓ Nothing needs you right now. No stuck, looping or long-waiting agents, no cross-pool traffic in the last hour, every computer reachable.</p>".into();
    }
    let mut h = toggle + "<ul class=\"alerts\">";
    for (i, al) in app.alerts.iter().enumerate() {
        let sev = ["crit", "warn", "info"][al.sev.min(2) as usize];
        let who = al.addr.as_deref().map(|a| app.name(a)).unwrap_or_default();
        h += &format!(
            "<li class=\"{sev}{}\" {}><span class=\"k\">{}</span><b>{}</b> <span class=\"muted\">{}</span><div class=\"what\">{}</div></li>",
            if i == app.alert_cursor.saturating_sub(1) && app.alert_cursor > 0 { " cur" } else { "" },
            al.addr.as_deref().map(|a| format!("data-action=\"focus\" data-addr=\"{}\"", esc(a))).unwrap_or_default(),
            al.kind,
            esc(&who),
            if al.at_ms > 0 { ago(al.at_ms) } else { String::new() },
            esc(&al.text)
        );
    }
    h += "</ul><p class=\"muted small\">Press <kbd>n</kbd> to jump through these on the map. Waiting means the process is alive but has been quiet for over 10 minutes; looping means ping-pong relay traffic or heavy edits to one file; stuck means marked busy but silent for 15 minutes.</p>";
    h
}

fn sort_key(app: &App, addr: &str, col: &str) -> (i64, String) {
    let a = &app.agents[addr];
    let i = app.insight(addr);
    match col {
        "name" => (0, app.name(addr).to_lowercase()),
        "kind" => (0, a.kind.map_or("", |k| k.as_str()).to_string()),
        "host" => (0, a.host.clone()),
        "health" => (i.map_or(9, |i| i.health as i64), String::new()),
        "tokens" => (a.tokens.fresh() as i64, String::new()),
        "cw" => (a.tokens.cache_write as i64, String::new()),
        "cr" => (a.tokens.cache_read as i64, String::new()),
        "burn" => (i.map_or(0, |i| i.burn_tpm as i64), String::new()),
        "cost" => (i.and_then(|i| i.cost).map_or(-1, |c| (c * 1000.0) as i64), String::new()),
        "files" => (a.files.len() as i64, String::new()),
        "msgs" => (i.map_or(0, |i| i.msgs as i64), String::new()),
        "subs" => (running_subs(app, addr) as i64, String::new()),
        _ => (a.last_active_ms, String::new()),
    }
}

/// Sub-agents and swarm members active in the last few minutes.
pub fn running_subs(app: &App, addr: &str) -> usize {
    let now = now_ms() as i64;
    let a = &app.agents[addr];
    let subs = a.subagents.iter().filter(|s| s.status != "done" && now - s.last_ms < 180_000).count();
    let kids = app.nodes.get(addr).map_or(0, |n| n.children.iter().filter(|c| app.agents.get(*c).map_or(false, |x| now - x.last_active_ms < 300_000)).count());
    subs + kids
}

/// A tiny tokens-per-minute sparkline for a table row.
fn mini_spark(app: &App, addr: &str) -> String {
    let v = insight::burn_series(app.tok_hist.get(addr), 15);
    let peak = v.iter().cloned().fold(0.0, f64::max);
    if peak <= 0.0 {
        return String::new();
    }
    let mut s = String::from("<svg class=\"spark\" viewBox=\"0 0 60 14\" preserveAspectRatio=\"none\">");
    for (i, x) in v.iter().enumerate() {
        if *x > 0.0 {
            let h = (x / peak * 13.0).max(1.0);
            s += &format!("<rect x=\"{}\" y=\"{:.1}\" width=\"3\" height=\"{h:.1}\" fill=\"{}\"/>", i * 4, 14.0 - h, rgba(heat(*x, 2_000.0, 2_000_000.0), 0.9));
        }
    }
    s + "</svg>"
}

fn agents_table(app: &App) -> String {
    let mut rows = app.ordered_matches();
    let (col, desc) = (&app.sort.0, app.sort.1);
    rows.sort_by(|x, y| {
        let o = sort_key(app, x, col).cmp(&sort_key(app, y, col));
        if desc {
            o.reverse()
        } else {
            o
        }
    });
    // Pinned agents first.
    rows.sort_by_key(|r| !app.pins.contains(r));
    let head = |id: &str, label: &str| {
        let arrow = if col == id { if desc { " ▾" } else { " ▴" } } else { "" };
        format!("<th data-sort=\"{id}\" class=\"{}\">{label}{arrow}</th>", if col == id { "on" } else { "" })
    };
    let mut total_cost = 0.0;
    let mut total_tok = relay_map_core::Tokens::default();
    let (mut burn, mut burn_cache) = (0.0, 0.0);
    let mut h = format!(
        "<table class=\"fleet\"><thead><tr><th></th>{}<th>{} · {}</th>{}{}{}{}{}{}<th>15 min</th>{}{}{}{}</tr></thead><tbody>",
        head("name", "Agent"),
        head("kind", "Type").replace("<th ", "<span ").replace("</th>", "</span>"),
        head("host", "Computer").replace("<th ", "<span ").replace("</th>", "</span>"),
        head("health", "Status"),
        head("active", "Active"),
        head("tokens", "Fresh").replace("<th ", "<th title=\"Input + output tokens, without cache\" "),
        head("cw", "Cache W").replace("<th ", "<th title=\"Cache write (cache creation) tokens\" "),
        head("cr", "Cache R").replace("<th ", "<th title=\"Cache read tokens: re-reading context already cached, billed at a tenth of input\" "),
        head("burn", "Tok/min").replace("<th ", "<th title=\"Fresh tokens per minute over the last 10 min; cache traffic shown underneath\" "),
        head("cost", "Cost"),
        head("files", "Files"),
        head("msgs", "Msgs"),
        head("subs", "Subs")
    );
    for addr in rows.iter().take(400) {
        let a = &app.agents[addr];
        let Some(i) = app.insight(addr) else { continue };
        total_tok.add(&a.tokens);
        total_cost += i.cost.unwrap_or(0.0);
        burn += i.burn_tpm;
        burn_cache += i.burn_cw_tpm + i.burn_cr_tpm;
        let sel = app.selection == Selection::Agent(addr.clone());
        h += &format!(
            "<tr data-action=\"focus\" data-addr=\"{a_}\" class=\"{}\"><td><span class=\"pin{}\" data-action=\"pin\" data-addr=\"{a_}\" title=\"Watch\">{}</span></td><td class=\"nm\" title=\"{}\"><b>{}</b><div class=\"muted\">{}</div></td><td class=\"wh\">{}<div style=\"color:{}\">{}</div></td><td>{}</td><td>{}</td><td class=\"n\">{}</td><td class=\"n muted\">{}</td><td class=\"n muted\">{}</td><td class=\"n\" title=\"{}\">{}</td><td>{}</td><td class=\"n\" title=\"{}\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td></tr>",
            if sel { "sel" } else { "" },
            if app.pins.contains(addr) { " on" } else { "" },
            if app.pins.contains(addr) { "★" } else { "☆" },
            esc(&format!("{}\n{}", a.title.as_deref().unwrap_or(""), a.cwd.as_deref().unwrap_or(""))),
            esc(&truncate(&app.name(addr), 34)),
            esc(&truncate(a.focus.as_deref().or(a.title.as_deref()).unwrap_or(""), 60)),
            kind_short(a.kind),
            host_color(&a.host),
            esc(&a.host),
            health_pill(i.health),
            if a.last_active_ms > 0 { ago(a.last_active_ms) } else { "—".into() },
            big(a.tokens.fresh()),
            dot_or_big(a.tokens.cache_write),
            dot_or_big(a.tokens.cache_read),
            format!(
                "fresh {}/min · cache write {}/min · cache read {}/min",
                big(i.burn_tpm as u64),
                big(i.burn_cw_tpm as u64),
                big(i.burn_cr_tpm as u64)
            ),
            burn_cell(i),
            mini_spark(app, addr),
            cost_title(a, i),
            i.cost.map(|c| format!("{}{}", insight::usd(c), if i.cost_estimated { "<sup>est</sup>" } else { "" })).unwrap_or_else(|| "—".into()),
            if a.files.is_empty() { "·".into() } else { a.files.len().to_string() },
            if i.msgs == 0 { "·".into() } else { i.msgs.to_string() },
            match running_subs(app, addr) {
                0 => "·".to_string(),
                n => format!("<b class=\"live\">{n}</b>"),
            },
            a_ = esc(addr),
        );
    }
    h += &format!(
        "</tbody><tfoot><tr><td></td><td colspan=\"5\">{} agents{}</td><td class=\"n\">{}</td><td class=\"n muted\">{}</td><td class=\"n muted\">{}</td><td class=\"n\">{}</td><td></td><td class=\"n\">{}</td><td colspan=\"3\"></td></tr></tfoot></table>",
        rows.len(),
        if app.query.is_empty() { String::new() } else { format!(" matching <code>{}</code>", esc(&app.search)) },
        big(total_tok.fresh()),
        big(total_tok.cache_write),
        big(total_tok.cache_read),
        if burn_cache > 0.0 { format!("{}<div class=\"muted\">+{} cache</div>", big(burn as u64), big(burn_cache as u64)) } else { big(burn as u64) },
        insight::usd(total_cost)
    );
    h += "<p class=\"muted small\">Filter with the search box: words, or <code>kind:codex</code> <code>host:laptop</code> <code>status:waiting</code> <code>status:alert</code> <code>project:…</code> <code>model:opus</code> <code>has:git</code>. Click a column to sort. Fresh is input + output; cache writes and reads are counted apart, so an agent re-reading a big cached context no longer looks like a heavy burner. Cost is the agent's own figure, or <sup>est</sup>imated from Claude list prices (hover for the split); burn counts since this page opened.</p>";
    h
}

fn dot_or_big(n: u64) -> String {
    if n == 0 {
        "·".into()
    } else {
        big(n)
    }
}

/// Fresh tokens per minute, with cache traffic on a muted second line.
fn burn_cell(i: &insight::Insight) -> String {
    let cache = i.burn_cw_tpm + i.burn_cr_tpm;
    match (i.burn_tpm > 0.0, cache > 0.0) {
        (false, false) => "·".into(),
        (_, false) => big(i.burn_tpm as u64),
        (_, true) => format!("{}<div class=\"muted\">+{} cache</div>", big(i.burn_tpm as u64), big(cache as u64)),
    }
}

/// Hover text for a cost cell: the list-price split by kind of token.
pub fn cost_title(a: &relay_map_core::Agent, i: &insight::Insight) -> String {
    match (insight::cost_parts(a), i.cost_estimated) {
        (Some(p), true) => format!(
            "input {} · output {} · cache write {} · cache read {}",
            insight::usd(p.input),
            insight::usd(p.output),
            insight::usd(p.cache_write),
            insight::usd(p.cache_read)
        ),
        (_, false) if i.cost.is_some() => "recorded by the agent".into(),
        _ => String::new(),
    }
}

fn messages(app: &App) -> String {
    let now = now_ms() as i64;
    let words: Vec<String> = app.query.words.clone();
    let sel = match &app.selection {
        Selection::Agent(a) => Some(a.clone()),
        _ => None,
    };
    let mut h = String::new();
    if let Some(s) = &sel {
        h += &format!("<p class=\"muted small\">Showing messages of <b>{}</b>. Press Esc to see everyone's.</p>", esc(&app.name(s)));
    }
    h += "<ul class=\"mlog\">";
    let mut n = 0;
    for m in app.messages.iter().rev().filter(|m| now - m.sent_ms < app.window_ms) {
        if m.source == MsgSource::Swarm && !app.show_swarm {
            continue;
        }
        if let Some(s) = &sel {
            if m.from_agent.as_deref() != Some(s) && m.to_agent.as_deref() != Some(s) {
                continue;
            }
        }
        let from = m.from_agent.as_deref().map(|a| app.name(a)).unwrap_or_else(|| m.from.clone());
        let to = m.to_agent.as_deref().map(|a| app.name(a)).unwrap_or_else(|| m.to.clone());
        if !words.is_empty() {
            let hay = format!("{from} {to} {} {} {}", m.subject.as_deref().unwrap_or(""), m.body, m.from).to_lowercase();
            if !words.iter().all(|w| hay.contains(w.as_str())) {
                continue;
            }
        }
        n += 1;
        if n > 300 {
            break;
        }
        let open = app.open_msgs.contains(&m.id);
        h += &format!(
            "<li data-action=\"toggle-msg\" data-a=\"{}\" data-b=\"{}\" data-id=\"{}\" class=\"{}{}{}\"><div class=\"hd\"><span class=\"t\">{}</span> <b>{}</b> → <b>{}</b> <span class=\"src\">{}</span><span class=\"sz\">{}</span></div><div class=\"what\">{}</div>{}</li>",
            esc(m.from_agent.as_deref().unwrap_or("")),
            esc(m.to_agent.as_deref().unwrap_or("")),
            esc(&m.id),
            if app.is_cross(m) { "cross " } else { "" },
            if m.source == MsgSource::Swarm { "swarm " } else { "" },
            if open { "open" } else { "" },
            day_clock(m.sent_ms),
            esc(&truncate(&from, 26)),
            esc(&truncate(&to, 26)),
            src_label(m.source),
            bytes(m.bytes),
            esc(&m.subject.clone().unwrap_or_else(|| truncate(&m.body.replace('\n', " "), 140))),
            if open {
                format!(
                    "<pre>{}</pre><div class=\"acts\"><a data-action=\"channel\" data-a=\"{}\" data-b=\"{}\">open the conversation ⇄</a> <code>{}</code> → <code>{}</code></div>",
                    esc(&m.body),
                    esc(m.from_agent.as_deref().unwrap_or("")),
                    esc(m.to_agent.as_deref().unwrap_or("")),
                    esc(&m.from),
                    esc(&m.to)
                )
            } else {
                String::new()
            }
        );
    }
    if n == 0 {
        h += "<li class=\"muted\">No messages match.</li>";
    }
    h += "</ul>";
    h
}

pub fn src_label(s: MsgSource) -> &'static str {
    match s {
        MsgSource::Inbox => "mailbox",
        MsgSource::Received => "delivered",
        MsgSource::Sent => "sent",
        MsgSource::Broadcast => "broadcast",
        MsgSource::Swarm => "swarm",
    }
}

pub fn render_watch(app: &App) {
    let mut pins: Vec<&String> = app.pins.iter().filter(|p| app.agents.contains_key(*p)).collect();
    if pins.is_empty() {
        show("watch", false);
        return;
    }
    pins.sort_by_key(|p| std::cmp::Reverse(app.agents[*p].last_active_ms));
    let mut h = String::from("<h3>★ WATCHING</h3><ul>");
    for p in pins {
        let a = &app.agents[p];
        let Some(i) = app.insight(p) else { continue };
        h += &format!(
            "<li data-action=\"focus\" data-addr=\"{}\"><span class=\"dot\" style=\"background:{}\"></span><b>{}</b> <span class=\"muted\">{} · {}{}</span><div class=\"what\">{}</div></li>",
            esc(p),
            rgba(i.health.color(), 1.0),
            esc(&truncate(&app.name(p), 28)),
            i.health.label(),
            ago(a.last_active_ms),
            if i.burn_tpm > 0.0 { format!(" · {}/min", big(i.burn_tpm as u64)) } else { String::new() },
            esc(&truncate(a.focus.as_deref().or(a.title.as_deref()).unwrap_or(""), 70))
        );
    }
    h += "</ul>";
    set_html("watch", &h);
    show("watch", true);
}

pub const BINS: usize = 72;

/// Traffic over the window in the strip at the bottom: one bar per slice,
/// cross-pool traffic in red, scoped to the selected agent when there is one.
pub fn render_timeline(app: &App) {
    let now = now_ms() as i64;
    let from = now - app.window_ms;
    let step = (app.window_ms as f64 / BINS as f64).max(1.0);
    let sel: Option<HashSet<String>> = match &app.selection {
        Selection::Agent(a) => Some([a.clone()].into_iter().collect()),
        Selection::Channel(a, b) => Some([a.clone(), b.clone()].into_iter().collect()),
        Selection::None => None,
    };
    let mut bins = vec![(0usize, 0usize, 0usize); BINS];
    for m in app.messages.iter().filter(|m| m.sent_ms >= from) {
        if let Some(s) = &sel {
            let f = m.from_agent.as_ref().map_or(false, |x| s.contains(x) || app.drawn_as.get(x).map_or(false, |d| s.contains(d)));
            let t = m.to_agent.as_ref().map_or(false, |x| s.contains(x) || app.drawn_as.get(x).map_or(false, |d| s.contains(d)));
            if !(f || t) {
                continue;
            }
        }
        let i = (((m.sent_ms - from) as f64 / step) as usize).min(BINS - 1);
        if m.source == MsgSource::Swarm {
            if app.show_swarm {
                bins[i].2 += 1;
            }
        } else if app.is_cross(m) {
            bins[i].1 += 1;
        } else {
            bins[i].0 += 1;
        }
    }
    let peak = bins.iter().map(|b| b.0 + b.1 + b.2).max().unwrap_or(0).max(1) as f64;
    let total: usize = bins.iter().map(|b| b.0 + b.1).sum();
    let replay_at = app.replay.as_ref().map(|r| {
        let frac = ((now_ms() - r.start_real) / r.span_real).clamp(0.0, 1.0);
        r.from_ms + ((r.to_ms - r.from_ms) as f64 * frac) as i64
    });
    let mut h = format!(
        "<div class=\"tl-hd\"><span>TRAFFIC{}</span><span class=\"muted\">{total} relay msgs · click a bar to replay from there</span></div><div class=\"bars\">",
        match &app.selection {
            Selection::Agent(a) => format!(" · {}", esc(&truncate(&app.name(a), 24))),
            Selection::Channel(a, b) => format!(" · {} ⇄ {}", esc(&truncate(&app.name(a), 14)), esc(&truncate(&app.name(b), 14))),
            Selection::None => String::new(),
        }
    );
    for (i, (n, x, s)) in bins.iter().enumerate() {
        let t0 = from + (i as f64 * step) as i64;
        let tot = n + x + s;
        let pct = |v: usize| if v == 0 { 0.0 } else { (v as f64 / peak * 100.0).max(6.0) };
        let past = replay_at.map_or(false, |r| r >= t0);
        h += &format!(
            "<div class=\"bar{}\" data-action=\"replay-from\" data-ms=\"{t0}\" title=\"{} · {} relay{}{}\"><i class=\"s\" style=\"height:{:.0}%\"></i><i class=\"x\" style=\"height:{:.0}%\"></i><i class=\"n\" style=\"height:{:.0}%\"></i></div>",
            if past { " past" } else { "" },
            day_clock(t0),
            n + x,
            if *x > 0 { format!(", {x} cross-pool") } else { String::new() },
            if *s > 0 { format!(", {s} swarm") } else { String::new() },
            pct(*s),
            pct(*x),
            pct(*n),
        );
        let _ = tot;
    }
    h += &format!("</div><div class=\"tl-ft muted\"><span>{}</span><span>now</span></div>", day_clock(from));
    set_html("timeline", &h);
}

/// Activity of one agent over the window as an inline SVG: messages, file
/// writes, git actions and sub-agent activity, one row each.
pub fn activity_svg(app: &App, addr: &str) -> String {
    let Some(a) = app.agents.get(addr) else { return String::new() };
    let now = now_ms() as i64;
    let from = now - app.window_ms;
    let bins = 60usize;
    let step = app.window_ms as f64 / bins as f64;
    let mut rows: Vec<(&str, &str, Vec<u32>)> = vec![
        ("msgs", "#4cb8ff", vec![0; bins]),
        ("files", "#46f5a0", vec![0; bins]),
        ("git", "#ff8a4c", vec![0; bins]),
        ("subs", "#c78bff", vec![0; bins]),
    ];
    let mut put = |row: usize, t: i64| {
        if t >= from && t <= now {
            let i = (((t - from) as f64 / step) as usize).min(bins - 1);
            rows[row].2[i] += 1;
        }
    };
    for m in app.messages_of(addr) {
        put(0, m.sent_ms);
    }
    for f in &a.files {
        put(1, f.at_ms);
    }
    for g in &a.git {
        put(2, g.at_ms);
    }
    for s in &a.subagents {
        put(3, s.started_ms);
        if s.last_ms != s.started_ms {
            put(3, s.last_ms);
        }
    }
    let (w, rh) = (440.0, 13.0);
    let cw = w / bins as f64;
    let mut svg = format!("<svg class=\"act\" viewBox=\"0 0 {} {}\" preserveAspectRatio=\"none\">", w + 40.0, rh * 4.0 + 14.0);
    for (r, (label, color, v)) in rows.iter().enumerate() {
        let y = r as f64 * rh;
        svg += &format!("<text x=\"0\" y=\"{:.1}\" class=\"lb\">{label}</text>", y + 9.5);
        svg += &format!("<line x1=\"40\" x2=\"{}\" y1=\"{:.1}\" y2=\"{:.1}\" class=\"gl\"/>", w + 40.0, y + 6.5, y + 6.5);
        let peak = *v.iter().max().unwrap_or(&1).max(&1) as f64;
        for (i, n) in v.iter().enumerate() {
            if *n == 0 {
                continue;
            }
            let k = 0.35 + 0.65 * (*n as f64 / peak);
            svg += &format!(
                "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" fill=\"{color}\" opacity=\"{k:.2}\"><title>{} {label} around {}</title></rect>",
                40.0 + i as f64 * cw,
                y + 1.5,
                (cw - 1.0).max(1.5),
                rh - 3.0,
                n,
                day_clock(from + (i as f64 * step) as i64)
            );
        }
    }
    // "last active" marker.
    if a.last_active_ms >= from {
        let x = 40.0 + (a.last_active_ms - from) as f64 / app.window_ms as f64 * w;
        svg += &format!("<line x1=\"{x:.1}\" x2=\"{x:.1}\" y1=\"0\" y2=\"{:.1}\" class=\"la\"/>", rh * 4.0);
    }
    svg += &format!("<text x=\"40\" y=\"{:.1}\" class=\"lb\">{}</text><text x=\"{}\" y=\"{:.1}\" class=\"lb\" text-anchor=\"end\">now</text></svg>", rh * 4.0 + 12.0, day_clock(from), w + 40.0, rh * 4.0 + 12.0);
    svg
}

/// Tokens per minute over the last 30 minutes as a sparkline.
pub fn burn_svg(app: &App, addr: &str) -> String {
    let v = insight::burn_series(app.tok_hist.get(addr), 30);
    let peak = v.iter().cloned().fold(0.0, f64::max);
    if peak <= 0.0 {
        return "<p class=\"muted small\">No fresh token use observed since this page opened.</p>".into();
    }
    let (w, h) = (440.0, 34.0);
    let cw = w / v.len() as f64;
    let mut s = format!("<svg class=\"burn\" viewBox=\"0 0 {w} {}\" preserveAspectRatio=\"none\">", h + 2.0);
    for (i, x) in v.iter().enumerate() {
        if *x <= 0.0 {
            continue;
        }
        let bh = (x / peak * h).max(1.5);
        s += &format!(
            "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{bh:.1}\" fill=\"{}\"><title>{} tokens, {} min ago</title></rect>",
            i as f64 * cw,
            h - bh,
            (cw - 1.0).max(1.0),
            rgba(heat(*x, 2_000.0, 2_000_000.0), 0.9),
            big(*x as u64),
            v.len() - 1 - i
        );
    }
    s += "</svg>";
    let cr: f64 = insight::cache_read_series(app.tok_hist.get(addr), 30).iter().sum();
    let note = if cr > 0.0 { format!("<span>fresh tokens · {} cache reads not drawn</span>", big(cr as u64)) } else { "<span>fresh tokens</span>".into() };
    format!("{s}<div class=\"muted small sp\"><span>30 min ago</span>{note}<span>peak {}/min</span><span>now</span></div>", big(peak as u64))
}

/// Spend split by project, computer, model and agent type, plus the top spenders.
fn spend(app: &App) -> String {
    use std::collections::HashMap;
    struct Row {
        cost: f64,
        est: bool,
        tokens: u64,
        usd_h: f64,
        agents: usize,
    }
    let addrs: Vec<&String> = app.drawn_as.keys().filter(|k| app.agents.get(*k).map_or(false, |a| a.kind != Some(AgentKind::Operator) && !a.ghost && app.matches_search(a))).collect();
    let model_of = |a: &Agent| {
        let m = a.model.clone().unwrap_or_else(|| "(unknown)".into());
        m.split('[').next().unwrap_or("").trim().to_string()
    };
    let dims: [(&str, Box<dyn Fn(&Agent) -> String>); 4] = [
        ("Project", Box::new(|a: &Agent| project_of(a.cwd.as_deref().unwrap_or("")))),
        ("Computer", Box::new(|a: &Agent| a.host.clone())),
        ("Model", Box::new(move |a: &Agent| model_of(a))),
        ("Agent type", Box::new(|a: &Agent| a.kind.map_or("other", |k| k.as_str()).to_string())),
    ];
    let (mut total, mut total_h, mut unpriced_tok, mut unpriced) = (0.0, 0.0, 0u64, 0usize);
    for k in &addrs {
        let i = app.insight(k);
        match i.and_then(|i| i.cost) {
            Some(c) => total += c,
            None => {
                unpriced += 1;
                unpriced_tok += app.agents[*k].tokens.fresh();
            }
        }
        total_h += i.map_or(0.0, |i| i.burn_usd_h);
    }
    let mut h = format!(
        "<div class=\"spend-top\"><div><b>{}</b><span>spent by {} agents on the map</span></div><div><b>{}</b><span>per hour right now</span></div><div><b>{}</b><span>fresh tokens in {} agents with no price</span></div></div>",
        insight::usd(total),
        addrs.len() - unpriced,
        if total_h > 0.0 { insight::usd(total_h) } else { "—".into() },
        big(unpriced_tok),
        unpriced
    );
    h += "<div class=\"spend-grid\">";
    for (title, f) in dims.iter() {
        let mut rows: HashMap<String, Row> = HashMap::new();
        for k in &addrs {
            let a = &app.agents[*k];
            let i = app.insight(k);
            let r = rows.entry(f(a)).or_insert(Row { cost: 0.0, est: false, tokens: 0, usd_h: 0.0, agents: 0 });
            r.cost += i.and_then(|i| i.cost).unwrap_or(0.0);
            r.est |= i.map_or(false, |i| i.cost_estimated);
            r.tokens += a.tokens.fresh();
            r.usd_h += i.map_or(0.0, |i| i.burn_usd_h);
            r.agents += 1;
        }
        let mut v: Vec<(String, Row)> = rows.into_iter().collect();
        v.sort_by(|a, b| b.1.cost.partial_cmp(&a.1.cost).unwrap().then(b.1.tokens.cmp(&a.1.tokens)));
        let peak = v.first().map_or(1.0, |x| x.1.cost.max(0.01));
        h += &format!("<section><h4>{title}</h4><ul class=\"bars\">");
        for (name, r) in v.iter().take(10) {
            let filter = match *title {
                "Project" => format!("project:{name}"),
                "Computer" => format!("host:{name}"),
                "Model" => format!("model:{name}"),
                _ => format!("kind:{name}"),
            };
            h += &format!(
                "<li data-action=\"filter\" data-q=\"{}\" title=\"Filter the map to {}\"><div class=\"lbl\"><span>{}</span><span class=\"muted\">{} agent{} · {} fresh tok{}</span><b>{}{}</b></div><div class=\"track\"><i style=\"width:{:.1}%\"></i></div></li>",
                esc(&filter),
                esc(name),
                esc(&truncate(name, 28)),
                r.agents,
                if r.agents == 1 { "" } else { "s" },
                big(r.tokens),
                if r.usd_h > 0.5 { format!(" · {}/h", insight::usd(r.usd_h)) } else { String::new() },
                if r.cost > 0.0 { insight::usd(r.cost) } else { "—".into() },
                if r.est && r.cost > 0.0 { "<sup>est</sup>" } else { "" },
                (r.cost / peak * 100.0).clamp(0.0, 100.0)
            );
        }
        h += "</ul></section>";
    }
    h += "</div><p class=\"muted small\">Cost is each agent's own figure, or <sup>est</sup>imated from Claude list prices (cache reads at 0.1x and writes at 1.25x the input price). Codex, Antigravity and pi sessions count only when they recorded a cost. Per-hour figures are measured since this page opened. Click a row to filter the map; the search box narrows this view too.</p>";
    h
}
