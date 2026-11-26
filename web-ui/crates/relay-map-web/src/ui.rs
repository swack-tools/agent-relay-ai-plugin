//! DOM side of the UI: header chips, live feed, detail panel, broadcast dialog.

use crate::app::*;
use crate::insight::{self, Health};
use crate::views;
use crate::util::*;
use relay_map_core::*;
use std::collections::BTreeMap;

pub fn toast_ok(text: &str) {
    if let Some(t) = el("toast") {
        let _ = t.class_list().add_1("ok");
    }
    set_html("toast", &esc(text));
    show("toast", true);
    let cb = wasm_bindgen::closure::Closure::once_into_js(|| show("toast", false));
    let _ = window().set_timeout_with_callback_and_timeout_and_arguments_0(cb.unchecked_ref(), 2500);
}

pub fn toast(text: &str) {
    if let Some(t) = el("toast") {
        let _ = t.class_list().remove_1("ok");
    }
    set_html("toast", &esc(text));
    show("toast", true);
    let cb = wasm_bindgen::closure::Closure::once_into_js(|| show("toast", false));
    let _ = window().set_timeout_with_callback_and_timeout_and_arguments_0(cb.unchecked_ref(), 5000);
}

use wasm_bindgen::JsCast;

fn name_of(app: &App, addr: &str) -> String {
    app.name(addr)
}

fn kind_badge(k: Option<AgentKind>) -> String {
    let (name, c) = match k {
        Some(k) => (k.as_str(), kind_color(Some(k))),
        None => ("agent", kind_color(None)),
    };
    format!("<span class=\"badge\" style=\"border-color:{0};color:{0}\">{1}</span>", rgba(c, 1.0), name)
}

pub fn render_hud(app: &App) {
    // Hosts.
    let mut s = String::new();
    for h in &app.hosts {
        let cls = match h.state.as_str() {
            "live" => "ok",
            "error" => "bad",
            _ => "warn",
        };
        s += &format!(
            "<span class=\"chip {cls}\" title=\"{}\"><i style=\"background:{}\"></i>{} <b>{}</b></span>",
            esc(&format!("{} via {}: {}", h.name, h.via, h.detail)),
            host_color(&h.name),
            esc(&h.name),
            esc(&h.state)
        );
    }
    if !app.connected {
        s += "<span class=\"chip bad\">hub disconnected, retrying…</span>";
    }
    set_html("hosts", &s);
    // Stats.
    let now = now_ms() as i64;
    let in_win: Vec<&RelayMessage> = app.messages.iter().filter(|m| now - m.sent_ms < app.window_ms).collect();
    let relay = in_win.iter().filter(|m| m.source != MsgSource::Swarm).count();
    let swarm = in_win.len() - relay;
    let cross = in_win.iter().filter(|m| app.is_cross(m)).count();
    let shown = app.nodes.len().saturating_sub(1);
    let folded: usize = app.nodes.values().map(|n| n.children.len()).sum();
    let mut counts = [0usize; 7];
    let (mut cost, mut tpm, mut usd_h) = (0.0, 0.0, 0.0);
    for k in app.drawn_as.keys() {
        if let Some(i) = app.insight(k) {
            counts[i.health as usize] += 1;
            cost += i.cost.unwrap_or(0.0);
            tpm += i.burn_tpm;
            usd_h += i.burn_usd_h;
        }
    }
    let pill = |h: Health, n: usize| {
        if n == 0 {
            String::new()
        } else {
            format!("<span class=\"hs\" data-status=\"{}\" title=\"Show only {} agents\"><i style=\"background:{}\"></i><b>{n}</b> {}</span>", h.id(), h.label(), rgba(h.color(), 1.0), h.label())
        }
    };
    set_html(
        "stats",
        &format!(
            "<span><b>{shown}</b> agents{}</span>{}{}{}{}<span title=\"Spend of the agents on the map: recorded, or estimated from Claude list prices\"><b>{}</b> spent</span><span title=\"Burn rate since this page opened\"><b>{}</b> tok/min{}</span><span><b>{relay}</b> relay msgs</span>{}{}",
            if folded > 0 { format!(" <em>(+{folded} in swarms)</em>") } else { String::new() },
            pill(Health::Working, counts[Health::Working as usize]),
            pill(Health::Waiting, counts[Health::Waiting as usize]),
            pill(Health::Stuck, counts[Health::Stuck as usize]),
            pill(Health::Looping, counts[Health::Looping as usize]),
            insight::usd(cost),
            big(tpm as u64),
            if usd_h > 0.5 { format!(" <em>≈{}/h</em>", insight::usd(usd_h)) } else { String::new() },
            if swarm > 0 && app.show_swarm { format!("<span><b>{swarm}</b> swarm</span>") } else { String::new() },
            if cross > 0 { format!("<span class=\"alert\"><b>{cross}</b> cross-pool</span>") } else { String::new() }
        ),
    );
    // Attention button and the tab title.
    let crit = app.alerts.iter().filter(|a| a.sev == 0).count();
    if let Some(b) = el("btn-attn") {
        b.set_inner_html(&format!("⚠ <b>{}</b>", app.alerts.len()));
        let _ = if crit > 0 { b.class_list().add_1("hot") } else { b.class_list().remove_1("hot") };
        let _ = b.set_attribute("title", &format!("{} need attention now, {} to look at (a)", crit, app.alerts.len() - crit));
    }
    document().set_title(&if crit > 0 { format!("({crit}) Relay Map") } else { "Relay Map".into() });
    // Search count.
    if app.query.is_empty() {
        set_html("search-count", "");
    } else {
        set_html("search-count", &format!("{} match", app.ordered_matches().len()));
    }
    // Panels sit under the header, whose height depends on how it wraps.
    if let (Some(top), Some(root)) = (el("top"), document().document_element()) {
        let b = top.get_bounding_client_rect().bottom();
        if let Ok(h) = root.dyn_into::<web_sys::HtmlElement>() {
            let _ = h.style().set_property("--top", &format!("{:.0}px", b));
        }
    }
    views::render_watch(app);
    views::render_timeline(app);
    // Feed.
    if app.feed_paused {
        return render_legend(app);
    }
    let mut f = String::new();
    for m in &app.feed {
        let from = m.from_agent.as_deref().map(|a| name_of(app, a)).unwrap_or_else(|| m.from.clone());
        let to = m.to_agent.as_deref().map(|a| name_of(app, a)).unwrap_or_else(|| m.to.clone());
        let what = m.subject.clone().unwrap_or_else(|| truncate(&m.body.replace('\n', " "), 70));
        let cross = app.is_cross(m);
        let pool = m.from_agent.as_ref().and_then(|a| app.agents.get(a)).and_then(|a| a.pool.clone());
        let col = if cross { parse_hex("#ff3b5c") } else { app.pool_color(pool.as_deref()) };
        let fresh = now - m.sent_ms < 4000 || app.replay.is_some();
        f += &format!(
            "<li data-action=\"channel\" data-a=\"{}\" data-b=\"{}\" class=\"{}{}\"><span class=\"t\">{}</span><span class=\"dot\" style=\"background:{}\"></span><b>{}</b> → <b>{}</b> <span class=\"sz\">{}</span><div class=\"what\">{}</div></li>",
            esc(m.from_agent.as_deref().unwrap_or("")),
            esc(m.to_agent.as_deref().unwrap_or("")),
            if cross { "cross" } else { "" },
            if fresh && app.feed.first().map(|x| &x.id) == Some(&m.id) { " fresh" } else { "" },
            clock(m.sent_ms),
            rgba(col, 1.0),
            esc(&truncate(&from, 22)),
            esc(&truncate(&to, 22)),
            bytes(m.bytes),
            esc(&what)
        );
    }
    if f.is_empty() {
        f = "<li class=\"empty\">No relay traffic in this window yet.</li>".into();
    }
    set_html("feed-list", &f);
    render_legend(app);
}

fn render_legend(app: &App) {
    let mut l = String::new();
    for p in app.pools.iter().filter(|p| !p.auto) {
        l += &format!(
            "<div><i style=\"background:{}\"></i>{}{}</div>",
            esc(&p.color),
            esc(&p.name),
            if p.isolated { " <em>isolated</em>" } else { "" }
        );
    }
    set_html("legend-pools", &l);
}

pub fn render_panel(app: &App) {
    match &app.selection {
        Selection::None => show("panel", false),
        Selection::Agent(addr) => {
            show("panel", true);
            set_html("panel-body", &agent_panel(app, addr));
        }
        Selection::Channel(a, b) => {
            show("panel", true);
            set_html("panel-body", &channel_panel(app, a, b));
        }
    }
}

fn agent_panel(app: &App, addr: &str) -> String {
    let Some(a) = app.agents.get(addr) else { return "<p>Agent not found.</p>".into() };
    let now = now_ms() as i64;
    let mut h = String::new();
    let pool = a.pool.clone().unwrap_or_else(|| UNPOOLED.into());
    let pcol = app.pool_color(a.pool.as_deref());
    let status = if a.ghost {
        "<span class=\"st ghost\">seen in messages only</span>".to_string()
    } else if a.alive == Some(true) {
        format!("<span class=\"st on\">● alive · {}</span>", esc(a.state.as_deref().unwrap_or("running")))
    } else if now - a.last_active_ms < 300_000 {
        "<span class=\"st on\">● active</span>".into()
    } else if a.alive == Some(false) {
        "<span class=\"st off\">○ process ended</span>".into()
    } else {
        "<span class=\"st idle\">◌ idle</span>".into()
    };
    let ins = app.insight(addr);
    let health = ins.map(|i| {
        let mut s = format!("<span class=\"hp\" style=\"color:{0};border-color:{0}\">{1}</span>", rgba(i.health.color(), 1.0), i.health.label());
        for r in &i.reasons {
            s += &format!(" <span class=\"why\">{}</span>", esc(r));
        }
        s
    });
    h += &format!(
        "<header><div class=\"row\">{}<span class=\"pool\" style=\"color:{}\">{}</span><span class=\"host\" style=\"color:{}\">@{}</span></div><h2>{}</h2>{} {}<code class=\"addr\" title=\"agent-relay address\">{}</code><div class=\"acts\"><button data-action=\"pin\" data-addr=\"{a_}\">{}</button><button data-action=\"copy\" data-text=\"{a_}\">Copy address</button><button data-action=\"copy\" data-text=\"{}\" title=\"A relay send command to paste into a terminal\">Copy relay send</button><button data-action=\"show-msgs\">Messages log</button><button data-action=\"open-log\" data-addr=\"{a_}\" title=\"Read this agent's full transcript: from this computer, over ssh, or VictoriaLogs\">Session log</button>{}</div></header>",
        kind_badge(a.kind),
        rgba(pcol, 1.0),
        esc(&pool),
        host_color(&a.host),
        esc(&a.host),
        esc(&app.name(addr)),
        status,
        health.unwrap_or_default(),
        esc(&a.addr),
        if app.pins.contains(addr) { "★ Watching" } else { "☆ Watch" },
        esc(&format!("relay send '{}' --subject '…' '…'", a.addr)),
        open_buttons(app, a),
        a_ = esc(addr),
    );
    if let Some(p) = &a.parent {
        h += &format!("<p class=\"muted\">Spawned by <a data-action=\"focus\" data-addr=\"{}\">{}</a></p>", esc(p), esc(&name_of(app, p)));
    }
    h += "<section class=\"work\">";
    h += &format!("<h4>Working on</h4><p class=\"title\">{}</p>", esc(a.title.as_deref().unwrap_or("(no task recorded)")));
    if let Some(f) = &a.focus {
        h += &format!("<h4>Right now</h4><p class=\"focus\">▸ {}</p>", esc(f));
    }
    if let Some(t) = &a.last_text {
        h += &format!("<details><summary>Last thing it said</summary><p class=\"said\">{}</p></details>", esc(t));
    }
    h += "</section>";
    let tk = &a.tokens;
    h += &format!(
        "<section class=\"grid\"><div><b>{}</b><span>input</span></div><div><b>{}</b><span>output</span></div><div><b>{}</b><span>{}</span></div><div class=\"cache\"><b>{}</b><span>cache write</span></div><div class=\"cache\"><b>{}</b><span>cache read</span></div><div><b>{}</b><span>last active</span></div></section>",
        big(tk.input),
        big(tk.output),
        ins.and_then(|i| i.cost).map(insight::usd).unwrap_or_else(|| big(tk.total())),
        match ins.map(|i| (i.cost.is_some(), i.cost_estimated)) {
            Some((true, true)) => "cost (est.)",
            Some((true, false)) => "cost",
            _ => "tokens in all",
        },
        big(tk.cache_write),
        big(tk.cache_read),
        ago(a.last_active_ms),
    );
    h += &token_split_bar(a, ins);
    if let Some(i) = ins {
        let session_min = ((a.last_active_ms - a.started_ms).max(60_000) as f64) / 60_000.0;
        h += &format!(
            "<section class=\"grid g2\"><div><b>{}</b><span>fresh tok/min · last 10 min</span></div><div class=\"cache\"><b>{}</b><span>cache tok/min · write + read</span></div><div><b>{}</b><span>spend/hour · last 10 min</span></div><div><b>{}</b><span>avg fresh tok/min</span></div><div><b>{}</b><span>started</span></div></section>",
            if i.burn_tpm > 0.0 { big(i.burn_tpm as u64) } else { "0".into() },
            if i.burn_cw_tpm + i.burn_cr_tpm > 0.0 { format!("{} + {}", big(i.burn_cw_tpm as u64), big(i.burn_cr_tpm as u64)) } else { "0".into() },
            if i.burn_usd_h > 0.0 { insight::usd(i.burn_usd_h) } else { "—".into() },
            if a.started_ms > 0 { big((a.tokens.fresh() as f64 / session_min) as u64) } else { "—".into() },
            if a.started_ms > 0 { day_clock(a.started_ms) } else { "—".into() }
        );
    }
    h += &format!("<details open><summary>Activity <em>{} window</em></summary>{}<h4>Token burn, last 30 min</h4>{}</details>", insight::dur(app.window_ms), views::activity_svg(app, addr), views::burn_svg(app, addr));
    let meta: Vec<String> = [a.model.as_ref().map(|m| format!("model {}", esc(m))), a.cwd.as_ref().map(|c| format!("<span title=\"{}\">{}</span>", esc(c), esc(&short_path(c, 3))))]
        .into_iter()
        .flatten()
        .collect();
    if !meta.is_empty() {
        h += &format!("<p class=\"meta\">{}</p>", meta.join(" · "));
    }
    // Sub-agents and threads.
    let kids: Vec<&String> = app.nodes.get(addr).map(|n| n.children.iter().collect()).unwrap_or_default();
    let n_sub = a.subagents.len() + kids.len();
    h += &format!("<details open><summary>Sub-agents &amp; threads <em>{n_sub}</em></summary><ul class=\"subs\">");
    for s in a.subagents.iter().rev().take(40) {
        let live = now - s.last_ms < 180_000 && s.status != "done";
        h += &format!(
            "<li><span class=\"st {}\">{}</span> <b>{}</b> <span class=\"muted\">{} · {}{}</span><div class=\"task\">{}</div></li>",
            if live { "on" } else { "idle" },
            if live { "●" } else { "○" },
            esc(&s.name),
            esc(&s.kind),
            ago(s.last_ms),
            match (s.tokens.saturating_sub(s.cache_read), s.cache_read) {
                (0, 0) => String::new(),
                (f, 0) => format!(" · {} tok", big(f)),
                (f, cr) => format!(" · {} tok + {} cache read", big(f), big(cr)),
            },
            esc(&truncate(&s.task, 260))
        );
    }
    for c in kids.iter().take(60) {
        if let Some(ca) = app.agents.get(*c) {
            let live = now - ca.last_active_ms < 300_000;
            h += &format!(
                "<li><span class=\"st {}\">{}</span> <a data-action=\"focus\" data-addr=\"{}\"><b>{}</b></a> <span class=\"muted\">swarm · {} · {} tok{}</span><div class=\"task\">{}</div></li>",
                if live { "on" } else { "idle" },
                if live { "●" } else { "○" },
                esc(c),
                esc(&ca.display_name()),
                ago(ca.last_active_ms),
                big(ca.tokens.fresh() + ca.tokens.cache_write),
                if ca.tokens.cache_read > 0 { format!(" + {} cache read", big(ca.tokens.cache_read)) } else { String::new() },
                esc(&truncate(ca.focus.as_deref().or(ca.title.as_deref()).unwrap_or(""), 220))
            );
        }
    }
    if n_sub == 0 {
        h += "<li class=\"muted\">None recorded.</li>";
    }
    h += "</ul></details>";
    // Files.
    let mut files = a.files.clone();
    files.sort_by_key(|f| -f.at_ms);
    let created = files.iter().filter(|f| f.op == FileOp::Created).count();
    h += &format!("<details open><summary>Files <em>{} touched · {} created</em></summary><ul class=\"files\">", files.len(), created);
    for f in files.iter().take(60) {
        let op = match f.op {
            FileOp::Created => "new",
            FileOp::Written => "write",
            FileOp::Edited => "edit",
            FileOp::Deleted => "del",
        };
        h += &format!(
            "<li><span class=\"op {op}\">{op}</span><span class=\"path\" title=\"{}\">{}</span><span class=\"muted\">{}{}</span></li>",
            esc(&f.path),
            esc(&short_path(&f.path, 3)),
            if f.count > 1 { format!("×{} · ", f.count) } else { String::new() },
            ago(f.at_ms)
        );
    }
    if files.is_empty() {
        h += "<li class=\"muted\">No file writes recorded.</li>";
    }
    h += "</ul></details>";
    // Git.
    let mut verbs: BTreeMap<String, usize> = BTreeMap::new();
    for g in &a.git {
        *verbs.entry(g.verb.clone()).or_default() += 1;
    }
    let mut vv: Vec<(String, usize)> = verbs.into_iter().collect();
    vv.sort_by_key(|x| std::cmp::Reverse(x.1));
    h += &format!("<details><summary>Git &amp; GitHub <em>{} actions</em></summary><div class=\"chips\">", a.git.len());
    for (v, n) in &vv {
        let hot = matches!(v.as_str(), "commit" | "push" | "merge" | "rebase" | "reset" | "gh pr create" | "gh pr merge" | "pr link");
        h += &format!("<span class=\"tag{}\">{} <b>{}</b></span>", if hot { " hot" } else { "" }, esc(v), n);
    }
    h += "</div><ul class=\"git\">";
    for g in a.git.iter().rev().take(25) {
        h += &format!("<li><code>{}</code><span class=\"muted\">{}</span></li>", esc(&g.command), ago(g.at_ms));
    }
    h += "</ul></details>";
    // Skills and tools.
    let mut sk: Vec<(&String, &u32)> = a.skills.iter().collect();
    sk.sort_by_key(|x| std::cmp::Reverse(*x.1));
    h += &format!("<details open><summary>Skills <em>{}</em></summary><div class=\"chips\">", sk.len());
    for (k, n) in &sk {
        h += &format!("<span class=\"tag skill\">{} <b>{}</b></span>", esc(k), n);
    }
    if sk.is_empty() {
        h += "<span class=\"muted\">None used.</span>";
    }
    h += "</div></details>";
    let mut tl: Vec<(&String, &u32)> = a.tools.iter().collect();
    tl.sort_by_key(|x| std::cmp::Reverse(*x.1));
    h += &format!("<details><summary>Tools <em>{} calls</em></summary><div class=\"chips\">", tl.iter().map(|x| *x.1 as u64).sum::<u64>());
    for (k, n) in tl.iter().take(24) {
        h += &format!("<span class=\"tag\">{} <b>{}</b></span>", esc(k), n);
    }
    h += "</div></details>";
    // Relay messages.
    let msgs = app.messages_of(addr);
    let relay: Vec<&&RelayMessage> = msgs.iter().filter(|m| m.source != MsgSource::Swarm).collect();
    h += &format!("<details open><summary>Relay messages <em>{} ({} swarm)</em></summary><ul class=\"msgs\">", relay.len(), msgs.len() - relay.len());
    for m in relay.iter().rev().take(30) {
        let out = m.from_agent.as_deref() == Some(addr);
        let peer = if out { m.to_agent.as_deref() } else { m.from_agent.as_deref() }.unwrap_or("");
        h += &format!(
            "<li data-action=\"channel\" data-a=\"{}\" data-b=\"{}\" class=\"{}\"><span class=\"dir\">{}</span> <b>{}</b> <span class=\"muted\">{} · {}</span><div class=\"what\">{}</div></li>",
            esc(addr),
            esc(peer),
            if app.is_cross(m) { "cross" } else { "" },
            if out { "→" } else { "←" },
            esc(&name_of(app, peer)),
            bytes(m.bytes),
            ago(m.sent_ms),
            esc(&m.subject.clone().unwrap_or_else(|| truncate(&m.body.replace('\n', " "), 120)))
        );
    }
    if relay.is_empty() {
        h += "<li class=\"muted\">No relay messages in the collected history.</li>";
    }
    h += "</ul></details>";
    if a.kind.map_or(false, |k| k.relay_target()) && !a.ghost {
        h += &format!("<button class=\"primary\" data-action=\"message-agent\" data-addr=\"{}\">Message this agent…</button>", esc(addr));
    }
    if let Some(t) = &a.transcript {
        h += &format!("<p class=\"muted small\">Transcript: <span title=\"{}\">{}</span></p>", esc(t), esc(&short_path(t, 2)));
    }
    h
}

fn channel_panel(app: &App, a: &str, b: &str) -> String {
    let msgs = app.messages_between(a, b);
    let mut h = String::new();
    let total: u64 = msgs.iter().map(|m| m.bytes).sum();
    let cross = msgs.iter().any(|m| app.is_cross(m));
    h += &format!(
        "<header><div class=\"row\"><span class=\"badge\">channel</span>{}</div><h2>{} ⇄ {}</h2><p class=\"muted\">{} messages · {}</p></header>",
        if cross { "<span class=\"badge alert\">crosses isolated pools</span>" } else { "" },
        esc(&name_of(app, a)),
        esc(&name_of(app, b)),
        msgs.len(),
        bytes(total)
    );
    h += "<section class=\"pair\">";
    for x in [a, b] {
        if let Some(ag) = app.agents.get(x) {
            h += &format!(
                "<div class=\"side\"><div class=\"row\">{} <a data-action=\"focus\" data-addr=\"{}\"><b>{}</b></a> <span class=\"host\" style=\"color:{}\">@{}</span></div><p class=\"title\">{}</p>{}</div>",
                kind_badge(ag.kind),
                esc(x),
                esc(&ag.display_name()),
                host_color(&ag.host),
                esc(&ag.host),
                esc(&truncate(ag.title.as_deref().unwrap_or("(no task recorded)"), 200)),
                ag.focus.as_ref().map(|f| format!("<p class=\"focus\">▸ {}</p>", esc(f))).unwrap_or_default()
            );
        }
    }
    h += "</section><h4>Conversation</h4><ul class=\"convo\">";
    for m in msgs.iter().rev().take(80) {
        let from = m.from_agent.as_deref().map(|x| name_of(app, x)).unwrap_or_else(|| m.from.clone());
        let right = m.from_agent.as_deref().map(|x| app.drawn_as.get(x).map(|s| s.as_str()).unwrap_or(x)) == Some(b);
        h += &format!(
            "<li class=\"{} {}\"><div class=\"hd\"><b>{}</b> <span class=\"muted\">{} · {} · {}</span></div>{}<details><summary>{}</summary><pre>{}</pre></details></li>",
            if right { "r" } else { "l" },
            if m.source == MsgSource::Swarm { "swarm" } else { "" },
            esc(&from),
            day_clock(m.sent_ms),
            bytes(m.bytes),
            match m.source {
                MsgSource::Inbox => "mailbox",
                MsgSource::Received => "delivered",
                MsgSource::Sent => "sent",
                MsgSource::Broadcast => "broadcast",
                MsgSource::Swarm => "swarm",
            },
            m.subject.as_ref().map(|s| format!("<div class=\"subj\">{}</div>", esc(s))).unwrap_or_default(),
            esc(&truncate(&m.body.replace('\n', " "), 140)),
            esc(&m.body)
        );
    }
    h += "</ul>";
    h
}

pub fn open_broadcast(app: &mut App, preset: Option<String>) {
    let mut opts = String::from("<option value=\"all\">Every live agent</option>");
    for p in &app.pools {
        let kind = if p.auto { "Computer · project" } else { "Pool" };
        opts += &format!("<option value=\"pool:{0}\">{1}: {0}</option>", esc(&p.name), kind);
    }
    if let Selection::Agent(a) = &app.selection {
        opts += &format!("<option value=\"agent:{}\">Selected: {}</option>", esc(a), esc(&name_of(app, a)));
    }
    if let Some(p) = &preset {
        if !matches!(&app.selection, Selection::Agent(a) if a == p) {
            opts += &format!("<option value=\"agent:{}\">Agent: {}</option>", esc(p), esc(&name_of(app, p)));
        }
    }
    set_html("bc-scope", &opts);
    if let Some(p) = preset {
        set_input_value("bc-scope", &format!("agent:{p}"));
    }
    app.broadcast_plan = None;
    app.broadcast_results.clear();
    app.broadcast_req = None;
    if let Some(b) = el("bc-send") {
        b.remove_attribute("data-armed").ok();
        b.set_inner_html("Send");
    }
    render_broadcast(app);
    show("modal", true);
}

pub fn scope_from_ui() -> Scope {
    let v = input_value("bc-scope");
    if let Some(p) = v.strip_prefix("pool:") {
        Scope::Pool { name: p.to_string() }
    } else if let Some(a) = v.strip_prefix("agent:") {
        Scope::Agents { addrs: vec![a.to_string()] }
    } else {
        Scope::All
    }
}

pub fn render_broadcast(app: &App) {
    let mut h = String::new();
    if let Some((targets, skipped)) = &app.broadcast_plan {
        h += &format!("<p><b>{}</b> recipient(s)</p><ul class=\"targets\">", targets.len());
        for t in targets {
            let res = app.broadcast_results.iter().find(|r| &r.addr == t);
            let (cls, note) = match res {
                Some(r) if r.ok => ("ok", format!("✓ {}", r.detail)),
                Some(r) => ("bad", format!("✗ {}", r.detail)),
                None if app.broadcast_req.is_some() && !app.broadcast_results.is_empty() => ("", "sending…".into()),
                None => ("", String::new()),
            };
            h += &format!("<li class=\"{cls}\"><b>{}</b> <code>{}</code><div class=\"muted\">{}</div></li>", esc(&name_of(app, t)), esc(t), esc(&note));
        }
        h += "</ul>";
        if !skipped.is_empty() {
            h += &format!("<details><summary>{} skipped</summary><ul class=\"muted\">", skipped.len());
            for s in skipped.iter().take(80) {
                h += &format!("<li>{}</li>", esc(s));
            }
            h += "</ul></details>";
        }
    } else {
        h += "<p class=\"muted\">Preview shows exactly who will receive it. Messages go through agent-relay (<code>relay send</code>), so Claude sessions get them at their next turn and idle Codex threads start a turn right away.</p>";
    }
    set_html("bc-plan", &h);
}

pub fn render_group_panel(app: &App) {
    use crate::group::{Level, ALL_LEVELS};
    let mut rows: Vec<(Level, bool)> = app.levels.iter().map(|l| (*l, true)).collect();
    for l in ALL_LEVELS {
        if !app.levels.contains(&l) {
            rows.push((l, false));
        }
    }
    let mut h = String::new();
    for (i, (l, on)) in rows.iter().enumerate() {
        h += &format!(
            "<li class=\"{}\"><input type=\"checkbox\" data-action=\"group-toggle\" data-level=\"{id}\" {}><span class=\"n\">{}</span><span class=\"t\">{}</span><button data-action=\"group-up\" data-level=\"{id}\" title=\"Move out (bigger circle)\" {}>↑</button><button data-action=\"group-down\" data-level=\"{id}\" title=\"Move in (smaller circle)\" {}>↓</button></li>",
            if *on { "" } else { "off" },
            if *on { "checked" } else { "" },
            if *on { format!("{}", i + 1) } else { "–".into() },
            esc(l.title()),
            if *on && i > 0 { "" } else { "disabled" },
            if *on && i + 1 < app.levels.len() { "" } else { "disabled" },
            id = l.id(),
        );
    }
    set_html("group-list", &h);
}

/// Shell-quote for a single-quoted argument.
fn sq(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "._/-+:@".contains(c)) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// "Open in VS Code" and "Copy resume command" for agents with a folder.
fn open_buttons(app: &App, a: &Agent) -> String {
    let Some(cwd) = a.cwd.as_deref().filter(|c| c.starts_with('/')) else { return String::new() };
    let local = a.host == app.self_host;
    let mut h = String::new();
    let url = if local { format!("vscode://file{cwd}") } else { format!("vscode://vscode-remote/ssh-remote+{}{cwd}", a.host) };
    h += &format!("<a class=\"btn\" href=\"{}\" title=\"Open {} in VS Code{}\">Open in VS Code</a>", esc(&url), esc(cwd), if local { "" } else { " over ssh" });
    let resume = match a.kind {
        Some(AgentKind::Claude) => Some(format!("claude --resume {}", a.id)),
        Some(AgentKind::Codex) => Some(format!("codex resume {}", a.id)),
        Some(AgentKind::Opencode) => Some(format!("opencode -s {}", a.id)),
        _ => None,
    };
    if let Some(r) = resume {
        let inner = format!("cd {} && {r}", sq(cwd));
        let cmd = if local { inner } else if inner.contains('"') || inner.contains('$') { format!("ssh -t {} {}", a.host, sq(&inner)) } else { format!("ssh -t {} \"{inner}\"", a.host) };
        h += &format!(
            "<button data-action=\"copy\" data-text=\"{}\" title=\"{}\n\nPaste into a terminal to continue this session. If it is still running elsewhere, this starts a second copy.\">Copy resume command</button>",
            esc(&cmd),
            esc(&cmd)
        );
    }
    h += &format!("<button data-action=\"copy\" data-text=\"{}\">Copy folder</button>", esc(cwd));
    h
}

/// A bar of the agent's tokens by kind, with the list-price cost of each part
/// when there is one, so cached context reads as cheap rather than heavy.
fn token_split_bar(a: &relay_map_core::Agent, ins: Option<&insight::Insight>) -> String {
    let tk = &a.tokens;
    let total = tk.total();
    if total == 0 {
        return String::new();
    }
    let parts = insight::cost_parts(a).filter(|_| ins.map_or(false, |i| i.cost_estimated));
    let segs = [
        ("input", tk.input, parts.map(|p| p.input), "tk-in"),
        ("output", tk.output, parts.map(|p| p.output), "tk-out"),
        ("cache write", tk.cache_write, parts.map(|p| p.cache_write), "tk-cw"),
        ("cache read", tk.cache_read, parts.map(|p| p.cache_read), "tk-cr"),
    ];
    let mut bar = String::from("<div class=\"tk-bar\">");
    let mut legend = String::from("<div class=\"tk-legend muted small\">");
    for (label, n, usd, class) in segs {
        if n == 0 {
            continue;
        }
        let pct = n as f64 / total as f64 * 100.0;
        let cost = usd.map(|c| format!(" · {}", insight::usd(c))).unwrap_or_default();
        bar += &format!("<i class=\"{class}\" style=\"width:{pct:.2}%\" title=\"{label}: {} tokens ({pct:.0}%){cost}\"></i>", big(n));
        legend += &format!("<span><i class=\"{class}\"></i>{label} {pct:.0}%{cost}</span>");
    }
    bar + "</div>" + &legend + "</div>"
}
