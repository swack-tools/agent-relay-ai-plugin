//! The session log viewer: an agent's full transcript, fetched from the hub on
//! demand (`/api/transcript`), shown as readable turns with a raw download.

use crate::util::*;
use relay_map_core::{TranscriptEntry, TranscriptView};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

thread_local! {
    /// The agent whose log is open (or loading), so a late reply for another
    /// agent is dropped.
    static OPEN: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

fn enc(s: &str) -> String {
    js_sys::encode_uri_component(s).as_string().unwrap_or_default()
}

pub fn raw_url(addr: &str) -> String {
    format!("/api/transcript?addr={}&raw=1", enc(addr))
}

pub fn is_open() -> bool {
    el("log").map_or(false, |e| !e.class_list().contains("hidden"))
}

pub fn close() {
    OPEN.with(|o| *o.borrow_mut() = None);
    show("log", false);
}

fn head(name: &str, addr: &str, info: &str) -> String {
    format!(
        "<div class=\"lt\"><b>Session log</b> <span>{}</span><div class=\"muted small\">{info}</div></div><div class=\"lb\"><label><input type=\"checkbox\" data-action=\"log-toggle\" data-class=\"hide-tools\"> hide tools</label><label><input type=\"checkbox\" data-action=\"log-toggle\" data-class=\"hide-thinking\"> hide thinking</label><a class=\"btn\" href=\"{}\" download title=\"The whole log as JSON lines, as the agent wrote it\">Download raw</a><button data-action=\"log-close\" title=\"Close (Esc)\">✕</button></div>",
        esc(name),
        esc(&raw_url(addr)),
    )
}

pub fn open(addr: &str, name: &str) {
    OPEN.with(|o| *o.borrow_mut() = Some(addr.to_string()));
    show("log", true);
    set_html("log-head", &head(name, addr, "Fetching…"));
    set_html("log-body", "<p class=\"muted\">Looking for the transcript on this computer, over ssh, then in VictoriaLogs…</p>");
    let url = format!("/api/transcript?addr={}", enc(addr));
    let (a1, n1, a2, n2) = (addr.to_string(), name.to_string(), addr.to_string(), name.to_string());
    let on_resp = Closure::once(move |r: JsValue| {
        let r: web_sys::Response = r.unchecked_into();
        let (ok, status) = (r.ok(), r.status());
        let Ok(text) = r.text() else { return };
        let on_text = Closure::once(move |t: JsValue| {
            let body = t.as_string().unwrap_or_default();
            if OPEN.with(|o| o.borrow().as_deref() != Some(a1.as_str())) {
                return;
            }
            match (ok, serde_json::from_str::<TranscriptView>(&body)) {
                (true, Ok(v)) => render(&n1, &a1, &v),
                _ => failed(&n1, &a1, &format!("HTTP {status}: {body}")),
            }
        });
        let _ = text.then(&on_text);
        on_text.forget();
    });
    let on_err = Closure::once(move |e: JsValue| {
        failed(&n2, &a2, &format!("{e:?}"));
    });
    let _ = window().fetch_with_str(&url).then2(&on_resp, &on_err);
    on_resp.forget();
    on_err.forget();
}

fn failed(name: &str, addr: &str, why: &str) {
    set_html("log-head", &head(name, addr, "Not found"));
    set_html("log-body", &format!("<p>Could not read this agent's session log.</p><pre class=\"err\">{}</pre>", esc(why)));
}

fn mb(n: u64) -> String {
    if n >= 1 << 20 {
        format!("{:.1} MB", n as f64 / (1 << 20) as f64)
    } else {
        format!("{} KB", (n + 1023) / 1024)
    }
}

fn render(name: &str, addr: &str, v: &TranscriptView) {
    let src = match v.source.as_str() {
        "local" => "read on this computer".to_string(),
        "ssh" => format!("fetched over ssh from {}", v.host),
        "victorialogs" => "from VictoriaLogs".to_string(),
        s => s.to_string(),
    };
    let mut info = format!("{} · {} turns · {}", src, v.entries.len(), mb(v.bytes));
    if v.shown_bytes < v.bytes {
        info += &format!(" · showing the last {}", mb(v.shown_bytes));
    }
    if let Some(p) = &v.path {
        info += &format!(" · <span title=\"{}\">{}</span>", esc(p), esc(&short_path(p, 3)));
    }
    set_html("log-head", &head(name, addr, &info));
    let mut h = String::new();
    for n in &v.notes {
        h += &format!("<p class=\"muted small\">Skipped: {}</p>", esc(n));
    }
    if v.entries.is_empty() {
        h += "<p class=\"muted\">The log has no readable turns.</p>";
    }
    for e in &v.entries {
        h += &entry(e);
    }
    set_html("log-body", &h);
    if let Some(b) = el("log-body") {
        b.set_scroll_top(b.scroll_height());
    }
}

fn entry(e: &TranscriptEntry) -> String {
    let when = if e.t > 0 { day_clock(e.t) } else { String::new() };
    let text = esc(&e.text);
    match e.role.as_str() {
        "tool" | "result" | "thinking" => {
            let first = e.text.lines().next().unwrap_or("");
            let first: String = first.chars().take(120).collect();
            let label = match e.role.as_str() {
                "tool" => format!("⚙ {}", esc(e.name.as_deref().unwrap_or("tool"))),
                "result" => "↳ result".into(),
                _ => "… thinking".into(),
            };
            format!(
                "<details class=\"le r-{}\"><summary><b>{label}</b> <code>{}</code><span class=\"muted\">{when}</span></summary><pre>{text}</pre></details>",
                e.role,
                esc(&first)
            )
        }
        role => {
            let who = match role {
                "user" => "User",
                "assistant" => "Agent",
                _ => "System",
            };
            format!(
                "<div class=\"le r-{role}\"><div class=\"lh\"><b>{who}</b>{}<span class=\"muted\">{when}</span></div><pre>{text}</pre></div>",
                e.name.as_deref().map(|n| format!(" <code>{}</code>", esc(n))).unwrap_or_default()
            )
        }
    }
}

/// Toggle a display class (hide tools, hide thinking) on the viewer.
pub fn toggle(class: &str) {
    if let Some(l) = el("log") {
        let _ = l.class_list().toggle(class);
    }
}
