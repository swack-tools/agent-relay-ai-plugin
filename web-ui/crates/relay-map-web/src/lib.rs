//! Relay Map front end: Rust compiled to WebAssembly, drawing on a canvas.

mod app;
mod draw;
mod group;
mod insight;
mod logview;
mod notify;
mod ui;
mod util;
mod views;

use app::{App, ColorBy, Selection, Tab};
use relay_map_core::{truncate, ClientMsg, ServerMsg};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use util::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, KeyboardEvent, MessageEvent, MouseEvent, WebSocket, WheelEvent};

thread_local! {
    static APP: RefCell<App> = RefCell::new(App::new());
    static WS: RefCell<Option<WebSocket>> = const { RefCell::new(None) };
    static LAST_PANEL: RefCell<f64> = const { RefCell::new(0.0) };
    static LAST_HUD: RefCell<f64> = const { RefCell::new(0.0) };
    static LAST_DRAWER: RefCell<f64> = const { RefCell::new(0.0) };
    /// Messages typed while the hub connection was down, sent when it is back.
    static PENDING: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn with<R>(f: impl FnOnce(&mut App) -> R) -> R {
    APP.with(|a| f(&mut a.borrow_mut()))
}

fn listen<E: JsCast + wasm_bindgen::convert::FromWasmAbi + 'static>(target: &web_sys::EventTarget, ev: &str, f: impl FnMut(E) + 'static) {
    let cb = Closure::<dyn FnMut(E)>::new(f);
    target.add_event_listener_with_callback(ev, cb.as_ref().unchecked_ref()).ok();
    cb.forget();
}

fn on_id<E: JsCast + wasm_bindgen::convert::FromWasmAbi + 'static>(id: &str, ev: &str, f: impl FnMut(E) + 'static) {
    if let Some(e) = el(id) {
        listen(&e, ev, f);
    }
}

fn send(msg: &ClientMsg) {
    let Ok(text) = serde_json::to_string(msg) else { return };
    let sent = WS.with(|w| match w.borrow().as_ref() {
        Some(ws) if ws.ready_state() == WebSocket::OPEN => ws.send_with_str(&text).is_ok(),
        _ => false,
    });
    if !sent {
        PENDING.with(|p| p.borrow_mut().push(text));
        ui::toast("Not connected to the hub right now; reconnecting and will send as soon as it is back.");
        // Do not wait out a long back-off.
        let stale = WS.with(|w| w.borrow().as_ref().map_or(true, |ws| ws.ready_state() == WebSocket::CLOSED));
        if stale {
            connect(1000);
        }
    }
}

fn flush_pending() {
    let items: Vec<String> = PENDING.with(|p| p.borrow_mut().drain(..).collect());
    WS.with(|w| {
        if let Some(ws) = w.borrow().as_ref() {
            for t in items {
                let _ = ws.send_with_str(&t);
            }
        }
    });
}

fn connect(delay: i32) {
    let loc = window().location();
    let proto = if loc.protocol().unwrap_or_default() == "https:" { "wss" } else { "ws" };
    let url = format!("{proto}://{}/ws", loc.host().unwrap_or_default());
    let Ok(ws) = WebSocket::new(&url) else { return };
    let onmsg = Closure::<dyn FnMut(MessageEvent)>::new(|e: MessageEvent| {
        let Some(text) = e.data().as_string() else { return };
        match serde_json::from_str::<ServerMsg>(&text) {
            Ok(m) => with(|a| {
                a.connected = true;
                a.ingest(m)
            }),
            Err(err) => web_sys::console::warn_1(&format!("bad message: {err}").into()),
        }
    });
    ws.set_onmessage(Some(onmsg.as_ref().unchecked_ref()));
    onmsg.forget();
    let onopen = Closure::<dyn FnMut()>::new(|| {
        with(|a| {
            a.connected = true;
            a.hud_dirty = true;
        });
        flush_pending();
    });
    ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
    onopen.forget();
    let onclose = Closure::<dyn FnMut()>::new(move || {
        with(|a| {
            a.connected = false;
            a.hud_dirty = true;
        });
        let next = (delay * 2).clamp(1000, 15000);
        let cb = Closure::once_into_js(move || connect(next));
        let _ = window().set_timeout_with_callback_and_timeout_and_arguments_0(cb.unchecked_ref(), next);
    });
    ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));
    onclose.forget();
    WS.with(|w| *w.borrow_mut() = Some(ws));
}

fn canvas() -> HtmlCanvasElement {
    el("map").unwrap().dyn_into().unwrap()
}

fn resize() {
    let c = canvas();
    let w = window();
    let (cw, ch) = (w.inner_width().unwrap().as_f64().unwrap(), w.inner_height().unwrap().as_f64().unwrap());
    let dpr = w.device_pixel_ratio().max(1.0);
    c.set_width((cw * dpr) as u32);
    c.set_height((ch * dpr) as u32);
    with(|a| {
        a.width = cw;
        a.height = ch;
        a.dpr = dpr;
    });
}

/// Node under a screen point, if any.
fn hit_node(a: &App, x: f64, y: f64) -> Option<String> {
    let z = a.cam.zoom.sqrt().clamp(0.45, 1.8);
    let mut best: Option<(f64, String)> = None;
    for (k, n) in &a.nodes {
        let (sx, sy) = a.world_to_screen(n.x, n.y);
        let d = ((sx - x).powi(2) + (sy - y).powi(2)).sqrt();
        if d < n.r * z + 6.0 && best.as_ref().map_or(true, |b| d < b.0) {
            best = Some((d, k.clone()));
        }
    }
    best.map(|b| b.1)
}

fn hit_channel(a: &App, x: f64, y: f64) -> Option<(String, String)> {
    let mut best: Option<(f64, (String, String))> = None;
    for c in &a.channels {
        if let Some(p) = draw::arc_points(a, &c.a, &c.b) {
            let d = draw::dist_to_arc(x, y, p);
            if d < 7.0 && best.as_ref().map_or(true, |b| d < b.0) {
                best = Some((d, (c.a.clone(), c.b.clone())));
            }
        }
    }
    best.map(|b| b.1)
}

fn tooltip(a: &App, x: f64, y: f64) {
    let Some(h) = &a.hover else {
        show("tooltip", false);
        return;
    };
    let Some(ag) = a.agents.get(h) else { return };
    let kids = a.nodes.get(h).map_or(0, |n| n.children.len());
    let text = format!(
        "<b>{}</b> <span class=\"muted\">{} @{}</span><br>{}<br><span class=\"muted\">{} tokens{} · active {}{}</span>",
        esc(&a.name(h)),
        ag.kind.map(|k| k.as_str()).unwrap_or("agent"),
        esc(&ag.host),
        esc(&truncate(ag.focus.as_deref().or(ag.title.as_deref()).unwrap_or(""), 140)),
        big(ag.tokens.fresh() + ag.tokens.cache_write),
        if ag.tokens.cache_read > 0 { format!(" + {} cache read", big(ag.tokens.cache_read)) } else { String::new() },
        ago(ag.last_active_ms),
        if kids > 0 { format!(" · {kids} swarm agents") } else { String::new() }
    );
    set_html("tooltip", &text);
    if let Some(t) = html("tooltip") {
        let s = t.style();
        let left = (x + 16.0).min(a.width - 330.0);
        let _ = s.set_property("left", &format!("{left}px"));
        let _ = s.set_property("top", &format!("{}px", y + 12.0));
    }
    show("tooltip", true);
}

fn clear_selection() {
    with(|a| {
        a.selection = Selection::None;
        a.panel_dirty = true;
        a.fit_all();
    });
}

fn click_action(target: web_sys::Element) {
    let mut cur = Some(target);
    while let Some(e) = cur {
        if let Some(action) = e.get_attribute("data-action") {
            match action.as_str() {
                "focus" => {
                    if let Some(addr) = e.get_attribute("data-addr") {
                        with(|a| a.focus(&addr));
                    }
                }
                "channel" => {
                    let (x, y) = (e.get_attribute("data-a").unwrap_or_default(), e.get_attribute("data-b").unwrap_or_default());
                    with(|a| {
                        let xa = a.drawn_as.get(&x).cloned().unwrap_or(x.clone());
                        let yb = a.drawn_as.get(&y).cloned().unwrap_or(y.clone());
                        if xa == yb || yb.is_empty() {
                            a.focus(&x);
                        } else {
                            let (p, q) = if xa < yb { (xa, yb) } else { (yb, xa) };
                            a.focus_channel(&p, &q);
                        }
                    });
                }
                "message-agent" => {
                    let addr = e.get_attribute("data-addr");
                    with(|a| ui::open_broadcast(a, addr));
                }
                "pin" => {
                    if let Some(addr) = e.get_attribute("data-addr") {
                        toggle_pin(&addr);
                    }
                }
                "copy" => {
                    if let Some(t) = e.get_attribute("data-text") {
                        copy_text(&t);
                    }
                }
                "show-msgs" => open_drawer(Tab::Messages),
                "filter" => {
                    if let Some(q) = e.get_attribute("data-q") {
                        set_search_text(&q);
                    }
                }
                "notify" => toggle_notify(),
                "open-log" => {
                    if let Some(addr) = e.get_attribute("data-addr") {
                        let name = with(|a| a.name(&addr));
                        logview::open(&addr, &name);
                    }
                }
                "log-close" => logview::close(),
                "log-toggle" => {
                    if let Some(c) = e.get_attribute("data-class") {
                        logview::toggle(&c);
                    }
                }
                "toggle-msg" => {
                    // Clicking inside the expanded body (to select text) keeps it open.
                    let in_body = cur_target_is_body(&e);
                    if !in_body {
                        if let Some(id) = e.get_attribute("data-id") {
                            with(|a| {
                                if !a.open_msgs.remove(&id) {
                                    a.open_msgs.insert(id);
                                }
                                a.drawer_dirty = true;
                            });
                            render_drawer_now();
                        }
                    }
                }
                "replay-from" => {
                    if let Some(ms) = e.get_attribute("data-ms").and_then(|v| v.parse::<i64>().ok()) {
                        with(|a| {
                            let span = ((now_ms() as i64 - ms) as f64 / a.window_ms as f64 * 36_000.0).clamp(4000.0, 36_000.0);
                            a.start_replay_from(ms, span);
                        });
                    }
                }
                _ => {}
            }
            return;
        }
        cur = e.parent_element();
    }
}

thread_local! {
    static LAST_CLICK_TARGET: RefCell<Option<web_sys::Element>> = const { RefCell::new(None) };
}

fn cur_target_is_body(row: &web_sys::Element) -> bool {
    LAST_CLICK_TARGET.with(|t| {
        let mut cur = t.borrow().clone();
        while let Some(e) = cur {
            if &e == row {
                return false;
            }
            if e.tag_name() == "PRE" || e.tag_name() == "A" {
                return true;
            }
            cur = e.parent_element();
        }
        false
    })
}

const PINS_KEY: &str = "relaymap.pins";
const NOTIFY_KEY: &str = "relaymap.notify";

/// Called from a desktop notification click.
pub fn focus_from_outside(addr: &str) {
    with(|a| a.focus(addr));
}

fn toggle_notify() {
    let on = with(|a| a.notify);
    if on {
        with(|a| a.notify = false);
        store_set(NOTIFY_KEY, "0");
        render_drawer_now();
        return;
    }
    notify::request(|granted| {
        with(|a| a.notify = granted);
        store_set(NOTIFY_KEY, if granted { "1" } else { "0" });
        if granted {
            ui::toast_ok("Desktop alerts on.");
        } else {
            ui::toast("The browser did not allow notifications for this page.");
        }
        render_drawer_now();
    });
}

/// Keep the search, drawer tab, colour mode and selected agent in the address
/// bar so a link (or a reload) opens the same view.
fn sync_hash(a: &App) {
    // Leave an incoming link alone until the map has applied it.
    if a.first_snapshot || a.nodes.len() <= 1 {
        return;
    }
    let mut parts: Vec<String> = Vec::new();
    let enc = |s: &str| js_sys::encode_uri_component(s).as_string().unwrap_or_default();
    if let Selection::Agent(x) = &a.selection {
        parts.push(format!("agent={}", enc(x)));
    }
    if !a.search.is_empty() {
        parts.push(format!("q={}", enc(&a.search)));
    }
    if let Some(t) = a.drawer {
        parts.push(format!("tab={}", t.id()));
    }
    if a.color_by != ColorBy::Kind {
        parts.push(format!("color={}", a.color_by.id()));
    }
    let want = if parts.is_empty() { String::new() } else { format!("#{}", parts.join("&")) };
    let loc = window().location();
    if loc.hash().unwrap_or_default() != want {
        let url = format!("{}{}{want}", loc.pathname().unwrap_or_default(), loc.search().unwrap_or_default());
        if let Ok(h) = window().history() {
            let _ = h.replace_state_with_url(&JsValue::NULL, "", Some(&url));
        }
    }
}

/// Apply q, tab, color and (when `live`) agent from the address bar.
fn apply_hash(live: bool) {
    let params = hash_params();
    let get = |k: &str| params.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone());
    let q = get("q").unwrap_or_default();
    set_input_value("search", &q);
    with(|a| a.set_search(&q));
    // On load a link without a colour keeps the viewer's saved choice.
    match get("color") {
        Some(c) => set_color(ColorBy::from_id(&c)),
        None if live => set_color(ColorBy::Kind),
        None => {}
    }
    let tab = get("tab").as_deref().and_then(Tab::from_id);
    with(|a| a.drawer = tab);
    let body = document().body().unwrap();
    let _ = if tab.is_some() { body.class_list().add_1("drawer-open") } else { body.class_list().remove_1("drawer-open") };
    if live {
        match get("agent") {
            Some(x) => with(|a| {
                let x = x.to_lowercase();
                let hit = a.nodes.keys().filter(|k| k.to_lowercase() == x || a.name(k).to_lowercase().contains(&x) || k.to_lowercase().contains(&x)).max_by_key(|k| a.agents.get(*k).map_or(0, |g| g.last_active_ms)).cloned();
                if let Some(h) = hit {
                    a.focus(&h);
                }
            }),
            None => clear_selection(),
        }
        refit();
    }
    render_drawer_now();
}

fn hash_params() -> Vec<(String, String)> {
    let hash = window().location().hash().unwrap_or_default();
    hash.trim_start_matches('#')
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.to_string(), js_sys::decode_uri_component(v).ok().and_then(|s| s.as_string()).unwrap_or_default()))
        .collect()
}
const COLOR_KEY: &str = "relaymap.color";

fn store_get(k: &str) -> Option<String> {
    window().local_storage().ok().flatten().and_then(|s| s.get_item(k).ok().flatten())
}

fn store_set(k: &str, v: &str) {
    if let Ok(Some(s)) = window().local_storage() {
        let _ = s.set_item(k, v);
    }
}

fn toggle_pin(addr: &str) {
    let list = with(|a| {
        if !a.pins.remove(addr) {
            a.pins.insert(addr.to_string());
        }
        a.panel_dirty = true;
        a.drawer_dirty = true;
        a.hud_dirty = true;
        a.pins.iter().cloned().collect::<Vec<_>>()
    });
    store_set(PINS_KEY, &list.join("\n"));
    render_drawer_now();
}

fn copy_text(t: &str) {
    let _ = window().navigator().clipboard().write_text(t);
    ui::toast_ok(&format!("Copied: {}", truncate(t, 80)));
}

fn set_color(c: ColorBy) {
    with(|a| a.color_by = c);
    set_input_value("color-by", c.id());
    store_set(COLOR_KEY, c.id());
}

fn open_drawer(t: Tab) {
    let same = with(|a| a.drawer == Some(t));
    with(|a| a.drawer = if same { None } else { Some(t) });
    let open = !same;
    let _ = if open { document().body().unwrap().class_list().add_1("drawer-open") } else { document().body().unwrap().class_list().remove_1("drawer-open") };
    refit();
    render_drawer_now();
}

fn close_drawer() {
    with(|a| a.drawer = None);
    let _ = document().body().unwrap().class_list().remove_1("drawer-open");
    refit();
    render_drawer_now();
}

/// Re-frame the map after the free screen area changed.
fn refit() {
    with(|a| match a.selection.clone() {
        Selection::None => a.fit_all(),
        Selection::Agent(x) => a.focus(&x),
        Selection::Channel(p, q) => a.focus_channel(&p, &q),
    });
}

fn render_drawer_now() {
    LAST_DRAWER.with(|l| *l.borrow_mut() = now_ms());
    with(|a| {
        a.drawer_dirty = false;
        views::render_drawer(a);
    });
}

fn set_search_text(s: &str) {
    set_input_value("search", s);
    with(|a| a.set_search(s));
    render_drawer_now();
}

/// Move the selection through the visible agents that match the search.
fn step_agent(dir: i64) {
    with(|a| {
        let list = a.ordered_matches();
        if list.is_empty() {
            return;
        }
        let cur = match &a.selection {
            Selection::Agent(x) => list.iter().position(|k| k == x),
            _ => None,
        };
        let i = match cur {
            Some(i) => (i as i64 + dir).rem_euclid(list.len() as i64) as usize,
            None => if dir > 0 { 0 } else { list.len() - 1 },
        };
        let k = list[i].clone();
        a.focus(&k);
    });
}

fn next_alert() {
    with(|a| {
        let targets: Vec<String> = a.alerts.iter().filter_map(|x| x.addr.clone()).collect();
        if targets.is_empty() {
            ui::toast_ok("Nothing needs attention right now.");
            return;
        }
        let i = a.alert_cursor % targets.len();
        a.alert_cursor = i + 1;
        let k = targets[i].clone();
        a.focus(&k);
        a.drawer_dirty = true;
    });
}

fn new_request_id() -> String {
    format!("{:x}{:x}", now_ms() as u64, (js_sys::Math::random() * 1e9) as u64)
}

fn broadcast(dry_run: bool, body: String) {
    let rid = new_request_id();
    with(|a| {
        a.broadcast_req = Some(rid.clone());
        a.broadcast_results.clear();
        if dry_run {
            a.broadcast_plan = None;
        }
    });
    if dry_run {
        set_html("bc-plan", "<p class=\"muted\">Asking the hub who would receive it…</p>");
        // Say so if the hub never answers.
        let rid2 = rid.clone();
        let cb = Closure::once_into_js(move || {
            let waiting = with(|a| a.broadcast_req.as_deref() == Some(rid2.as_str()) && a.broadcast_plan.is_none());
            if waiting {
                set_html("bc-plan", "<p class=\"alert\">No answer from the hub. Check that <code>relay-map serve</code> is running, then reload this page.</p>");
            }
        });
        let _ = window().set_timeout_with_callback_and_timeout_and_arguments_0(cb.unchecked_ref(), 6000);
    }
    send(&ClientMsg::Broadcast {
        request_id: rid,
        scope: ui::scope_from_ui(),
        subject: input_value("bc-subject"),
        body,
        dry_run,
        include_stale: checked("bc-stale"),
    });
}

fn bind_ui() {
    let c = canvas();
    listen(&c, "mousedown", |e: MouseEvent| with(|a| a.drag = Some((e.client_x() as f64, e.client_y() as f64, a.cam.x, a.cam.y, false))));
    listen(&c, "mousemove", |e: MouseEvent| {
        let (x, y) = (e.client_x() as f64, e.client_y() as f64);
        with(|a| {
            if let Some((sx, sy, cx, cy, moved)) = a.drag {
                let moved = moved || (x - sx).abs() + (y - sy).abs() > 4.0;
                if moved {
                    a.cam.tx = cx - (x - sx) / a.cam.zoom;
                    a.cam.ty = cy - (y - sy) / a.cam.zoom;
                    a.cam.x = a.cam.tx;
                    a.cam.y = a.cam.ty;
                }
                a.drag = Some((sx, sy, cx, cy, moved));
                return;
            }
            let h = hit_node(a, x, y);
            let over_chan = h.is_none() && hit_channel(a, x, y).is_some();
            a.hover = h;
            canvas().style().set_property("cursor", if a.hover.is_some() || over_chan { "pointer" } else { "grab" }).ok();
            tooltip(a, x, y);
        });
    });
    listen(&c, "mouseup", |e: MouseEvent| {
        let (x, y) = (e.client_x() as f64, e.client_y() as f64);
        let click = with(|a| matches!(a.drag.take(), Some((_, _, _, _, false))));
        if !click {
            return;
        }
        let target = with(|a| (hit_node(a, x, y), hit_channel(a, x, y)));
        match target {
            (Some(n), _) => with(|a| a.focus(&n)),
            (None, Some((p, q))) => with(|a| a.focus_channel(&p, &q)),
            _ => clear_selection(),
        }
    });
    listen(&c, "mouseleave", |_: MouseEvent| {
        with(|a| {
            a.drag = None;
            a.hover = None;
        });
        show("tooltip", false);
    });
    listen(&c, "wheel", |e: WheelEvent| {
        e.prevent_default();
        let (x, y) = (e.client_x() as f64, e.client_y() as f64);
        with(|a| {
            let (wx, wy) = a.screen_to_world(x, y);
            let factor = (-e.delta_y() * 0.0015).exp();
            let nz = (a.cam.zoom * factor).clamp(0.12, 5.0);
            a.cam.zoom = nz;
            a.cam.tzoom = nz;
            // Keep the point under the cursor fixed.
            a.cam.x = wx - (x - a.width / 2.0) / nz;
            a.cam.y = wy - (y - a.height / 2.0) / nz;
            a.cam.tx = a.cam.x;
            a.cam.ty = a.cam.y;
        });
    });
    listen(&c, "dblclick", |_: MouseEvent| clear_selection());
    listen(&window(), "resize", |_: web_sys::Event| resize());
    // A link pasted into the address bar of an open page.
    listen(&window(), "hashchange", |_: web_sys::Event| apply_hash(true));
    listen(&window(), "keydown", |e: KeyboardEvent| {
        let tag = e.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()).map(|e| e.tag_name()).unwrap_or_default();
        if tag == "INPUT" || tag == "TEXTAREA" || tag == "SELECT" {
            if e.key() == "Escape" {
                show("modal", false);
            }
            return;
        }
        if e.meta_key() || e.ctrl_key() || e.alt_key() {
            return;
        }
        match e.key().as_str() {
            "Escape" => {
                if logview::is_open() {
                    logview::close();
                    return;
                }
                let help = el("help").map_or(false, |h| !h.class_list().contains("hidden"));
                let modal = el("modal").map_or(false, |h| !h.class_list().contains("hidden"));
                show("modal", false);
                show("help", false);
                if help || modal {
                    return;
                }
                let sel = with(|a| a.selection != Selection::None);
                if sel {
                    clear_selection();
                } else {
                    close_drawer();
                }
            }
            "/" => {
                e.prevent_default();
                if let Some(i) = el("search").and_then(|x| x.dyn_into::<web_sys::HtmlInputElement>().ok()) {
                    let _ = i.focus();
                    i.select();
                }
            }
            "?" => show("help", true),
            "j" | "ArrowDown" => step_agent(1),
            "k" | "ArrowUp" => step_agent(-1),
            "n" => next_alert(),
            "a" => open_drawer(Tab::Attention),
            "t" => open_drawer(Tab::Agents),
            "m" => open_drawer(Tab::Messages),
            "s" => open_drawer(Tab::Spend),
            "c" => {
                let c = with(|a| a.color_by.next());
                set_color(c);
            }
            "w" => {
                if let Some(Selection::Agent(x)) = with(|a| Some(a.selection.clone())) {
                    toggle_pin(&x);
                }
            }
            "y" => {
                if let Some(Selection::Agent(x)) = with(|a| Some(a.selection.clone())) {
                    copy_text(&x);
                }
            }
            "b" => with(|a| ui::open_broadcast(a, None)),
            "r" => with(|a| a.start_replay(36_000.0)),
            "f" => with(|a| a.fit_all()),
            "g" => toggle_group_panel(),
            _ => {}
        }
    });
    on_id("window", "change", |_: web_sys::Event| {
        let h: f64 = input_value("window").parse().unwrap_or(24.0);
        with(|a| {
            a.window_ms = (h * 3_600_000.0) as i64;
            a.layout_dirty = true;
            a.hud_dirty = true;
            a.panel_dirty = true;
        });
    });
    on_id("opt-unpooled", "change", |_: web_sys::Event| {
        with(|a| {
            a.show_unpooled = checked("opt-unpooled");
            a.layout_dirty = true;
        })
    });
    on_id("opt-swarm", "change", |_: web_sys::Event| {
        with(|a| {
            a.show_swarm = checked("opt-swarm");
            a.layout_dirty = true;
            a.hud_dirty = true;
        })
    });
    on_id("opt-expand", "change", |_: web_sys::Event| {
        with(|a| {
            a.expand_swarms = checked("opt-expand");
            a.layout_dirty = true;
        })
    });
    on_id("search", "input", |_: web_sys::Event| {
        let v = input_value("search");
        with(|a| a.set_search(&v));
        render_drawer_now();
    });
    on_id("btn-group", "click", |_: MouseEvent| toggle_group_panel());
    on_id("group-panel", "click", |e: MouseEvent| {
        let Some(t) = e.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) else { return };
        if let Some(p) = t.get_attribute("data-preset") {
            let lv: Vec<group::Level> = p.split(',').filter_map(group::Level::from_id).collect();
            set_levels(lv);
            return;
        }
        let (Some(action), Some(id)) = (t.get_attribute("data-action"), t.get_attribute("data-level")) else { return };
        let Some(level) = group::Level::from_id(&id) else { return };
        let mut lv = with(|a| a.levels.clone());
        match action.as_str() {
            "group-toggle" => {
                if let Some(i) = lv.iter().position(|l| *l == level) {
                    lv.remove(i);
                } else {
                    lv.push(level);
                }
            }
            "group-up" => {
                if let Some(i) = lv.iter().position(|l| *l == level).filter(|&i| i > 0) {
                    lv.swap(i, i - 1);
                }
            }
            "group-down" => {
                if let Some(i) = lv.iter().position(|l| *l == level).filter(|&i| i + 1 < lv.len()) {
                    lv.swap(i, i + 1);
                }
            }
            _ => return,
        }
        set_levels(lv);
    });
    on_id("btn-replay", "click", |_: MouseEvent| with(|a| a.start_replay(36_000.0)));
    on_id("btn-fit", "click", |_: MouseEvent| clear_selection());
    on_id("btn-broadcast", "click", |_: MouseEvent| with(|a| ui::open_broadcast(a, None)));
    on_id("panel-close", "click", |_: MouseEvent| clear_selection());
    for id in ["panel-body", "feed-list", "watch", "timeline", "log"] {
        on_id(id, "click", |e: MouseEvent| {
            if let Some(t) = e.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
                click_action(t);
            }
        });
    }
    on_id("drawer-body", "click", |e: MouseEvent| {
        let Some(t) = e.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) else { return };
        if let Some(col) = t.closest("[data-sort]").ok().flatten().and_then(|th| th.get_attribute("data-sort")) {
            with(|a| {
                if a.sort.0 == col {
                    a.sort.1 = !a.sort.1;
                } else {
                    // Text columns start A→Z, numbers start biggest first.
                    a.sort = (col.clone(), !matches!(col.as_str(), "name" | "kind" | "host" | "health"));
                }
            });
            render_drawer_now();
            return;
        }
        LAST_CLICK_TARGET.with(|l| *l.borrow_mut() = Some(t.clone()));
        click_action(t);
    });
    on_id("drawer", "click", |e: MouseEvent| {
        let Some(t) = e.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) else { return };
        let Some(b) = t.closest("[data-tab]").ok().flatten() else { return };
        let tab = b.get_attribute("data-tab").as_deref().and_then(Tab::from_id).unwrap_or(Tab::Attention);
        with(|a| a.drawer = Some(tab));
        render_drawer_now();
    });
    on_id("drawer", "mouseenter", |_: MouseEvent| with(|a| a.drawer_hover = true));
    on_id("drawer", "mouseleave", |_: MouseEvent| with(|a| a.drawer_hover = false));
    on_id("drawer-close", "click", |_: MouseEvent| close_drawer());
    for id in ["feed-list", "drawer-body"] {
        on_id(id, "mouseover", |e: MouseEvent| {
            let li = e.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()).and_then(|t| t.closest("li[data-a]").ok().flatten());
            with(|a| {
                a.hover_channel = li.and_then(|li| {
                    let x = li.get_attribute("data-a")?;
                    let y = li.get_attribute("data-b")?;
                    let xa = a.drawn_as.get(&x).cloned()?;
                    let yb = a.drawn_as.get(&y).cloned()?;
                    (xa != yb).then_some((xa, yb))
                });
            });
        });
        on_id(id, "mouseleave", |_: MouseEvent| with(|a| a.hover_channel = None));
    }
    on_id("btn-attn", "click", |_: MouseEvent| open_drawer(Tab::Attention));
    on_id("btn-table", "click", |_: MouseEvent| open_drawer(Tab::Agents));
    on_id("btn-help", "click", |_: MouseEvent| show("help", true));
    on_id("help-close", "click", |_: MouseEvent| show("help", false));
    on_id("help", "click", |e: MouseEvent| {
        if e.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()).map_or(false, |t| t.id() == "help") {
            show("help", false);
        }
    });
    on_id("color-by", "change", |_: web_sys::Event| set_color(ColorBy::from_id(&input_value("color-by"))));
    on_id("stats", "click", |e: MouseEvent| {
        let Some(t) = e.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) else { return };
        if let Some(st) = t.closest("[data-status]").ok().flatten().and_then(|x| x.get_attribute("data-status")) {
            let q = format!("status:{st}");
            let cur = input_value("search");
            set_search_text(if cur == q { "" } else { &q });
        }
    });
    on_id("search", "keydown", |e: KeyboardEvent| match e.key().as_str() {
        "Enter" => {
            let best = with(|a| a.ordered_matches().into_iter().next());
            if let Some(b) = best {
                with(|a| a.focus(&b));
            }
        }
        "Escape" => {
            if let Some(i) = el("search").and_then(|x| x.dyn_into::<web_sys::HtmlElement>().ok()) {
                let _ = i.blur();
            }
        }
        "ArrowDown" => step_agent(1),
        "ArrowUp" => step_agent(-1),
        _ => {}
    });
    on_id("bc-close", "click", |_: MouseEvent| show("modal", false));
    on_id("modal", "click", |e: MouseEvent| {
        if e.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()).map_or(false, |t| t.id() == "modal") {
            show("modal", false);
        }
    });
    on_id("feed", "mouseenter", |_: MouseEvent| with(|a| a.feed_paused = true));
    on_id("feed", "mouseleave", |_: MouseEvent| with(|a| {
        a.feed_paused = false;
        a.hud_dirty = true;
    }));
    on_id("bc-scope", "change", |_: web_sys::Event| {
        with(|a| {
            a.broadcast_plan = None;
            a.broadcast_results.clear();
            a.broadcast_req = None;
            ui::render_broadcast(a);
        })
    });
    on_id("bc-preview", "click", |_: MouseEvent| broadcast(true, input_value("bc-body")));
    on_id("bc-send", "click", |_: MouseEvent| {
        let body = input_value("bc-body");
        if body.trim().is_empty() {
            ui::toast("Write the message first.");
            return;
        }
        let Some(n) = with(|a| a.broadcast_plan.as_ref().map(|p| p.0.len())) else {
            // Show who it would go to; the next press confirms.
            broadcast(true, body);
            if let Some(b) = el("bc-send") {
                b.set_attribute("data-armed", "1").ok();
                b.set_inner_html("Confirm send");
            }
            ui::toast("Here is who would receive it. Press Confirm send to send.");
            return;
        };
        if n == 0 {
            ui::toast("Nobody to send to in that scope.");
            return;
        }
        // Two-step confirm inside the dialog (no browser pop-up).
        let armed = el("bc-send").map_or(false, |b| b.get_attribute("data-armed").is_some());
        if !armed {
            if let Some(b) = el("bc-send") {
                b.set_attribute("data-armed", "1").ok();
                b.set_inner_html(&format!("Confirm: send to {n}"));
            }
            return;
        }
        if let Some(b) = el("bc-send") {
            b.remove_attribute("data-armed").ok();
            b.set_inner_html("Send");
        }
        broadcast(false, body);
    });
}

const LEVELS_KEY: &str = "relaymap.levels";

fn toggle_group_panel() {
    let hidden = el("group-panel").map_or(true, |e| e.class_list().contains("hidden"));
    if hidden {
        with(|a| ui::render_group_panel(a));
    }
    show("group-panel", hidden);
}

fn set_levels(lv: Vec<group::Level>) {
    let ids: Vec<&str> = lv.iter().map(|l| l.id()).collect();
    if let Ok(Some(store)) = window().local_storage() {
        let _ = store.set_item(LEVELS_KEY, &ids.join(","));
    }
    with(|a| {
        a.levels = lv;
        a.layout_dirty = true;
        a.layout();
        a.fit_all();
        ui::render_group_panel(a);
    });
}

fn load_levels() {
    let saved = window().local_storage().ok().flatten().and_then(|s| s.get_item(LEVELS_KEY).ok().flatten());
    if let Some(saved) = saved {
        let lv: Vec<group::Level> = saved.split(',').filter_map(group::Level::from_id).collect();
        with(|a| a.levels = lv);
    }
}

fn summary_key(d: &web_sys::Element) -> String {
    let key = d.query_selector("summary").ok().flatten().and_then(|s| s.text_content()).unwrap_or_default();
    key.split_whitespace().take(2).collect::<Vec<_>>().join(" ")
}

/// Re-render the side panel without losing open sections or scroll position.
fn rerender_panel(a: &App) {
    let body = el("panel-body");
    let scroll = body.as_ref().map_or(0, |b| b.scroll_top());
    let same = body.as_ref().and_then(|b| b.get_attribute("data-sel")) == Some(format!("{:?}", sel_key(&a.selection)));
    let mut open: HashMap<String, bool> = HashMap::new();
    if let (Some(b), true) = (&body, same) {
        if let Ok(list) = b.query_selector_all("details") {
            for i in 0..list.length() {
                if let Some(d) = list.item(i).and_then(|n| n.dyn_into::<web_sys::Element>().ok()) {
                    open.insert(summary_key(&d), d.has_attribute("open"));
                }
            }
        }
    }
    ui::render_panel(a);
    if let Some(b) = &body {
        b.set_attribute("data-sel", &format!("{:?}", sel_key(&a.selection))).ok();
        if same {
            if let Ok(list) = b.query_selector_all("details") {
                for i in 0..list.length() {
                    if let Some(d) = list.item(i).and_then(|n| n.dyn_into::<web_sys::Element>().ok()) {
                        match open.get(&summary_key(&d)) {
                            Some(true) => d.set_attribute("open", "").ok(),
                            Some(false) => d.remove_attribute("open").ok(),
                            None => None,
                        };
                    }
                }
            }
            b.set_scroll_top(scroll);
        } else {
            b.set_scroll_top(0);
        }
    }
}

fn sel_key(s: &Selection) -> String {
    match s {
        Selection::None => String::new(),
        Selection::Agent(a) => a.clone(),
        Selection::Channel(a, b) => format!("{a}|{b}"),
    }
}

fn tick(ctx: &CanvasRenderingContext2d) {
    let t = now_ms();
    with(|a| {
        let relayout = (a.layout_dirty && t - a.last_layout > 250.0) || t - a.last_layout > 5000.0;
        if relayout {
            let first = a.nodes.len() <= 1;
            a.layout();
            if first && a.nodes.len() > 1 {
                a.fit_all();
                a.cam.zoom = a.cam.tzoom;
                a.cam.x = a.cam.tx;
                a.cam.y = a.cam.ty;
                // Deep link: #agent=<name, folder or address> zooms straight to it.
                if let Some(q) = hash_params().into_iter().find(|(k, _)| k == "agent").map(|(_, v)| v.to_lowercase()) {
                    let hit = a
                        .nodes
                        .keys()
                        .filter(|k| a.name(k).to_lowercase().contains(&q) || k.to_lowercase().contains(&q))
                        .max_by_key(|k| a.agents.get(*k).map_or(0, |x| x.tokens.total()))
                        .cloned();
                    if let Some(h) = hit {
                        a.focus(&h);
                    }
                }
            }
        }
        let k = 0.12;
        a.cam.x += (a.cam.tx - a.cam.x) * k;
        a.cam.y += (a.cam.ty - a.cam.y) * k;
        a.cam.zoom += (a.cam.tzoom - a.cam.zoom) * k;
        match a.tick_replay() {
            Some(ms) => {
                set_html("replay-clock", &format!("▶ REPLAY · {}", day_clock(ms)));
                show("replay-clock", true);
            }
            None => show("replay-clock", false),
        }
        if t - a.last_insight > 2000.0 {
            a.refresh_insights();
            a.hud_dirty = true;
            if matches!(a.selection, Selection::Agent(_)) {
                a.panel_dirty = true;
            }
        }
        draw::frame(a, ctx, t);
        let since_drawer = t - LAST_DRAWER.with(|l| *l.borrow());
        if a.drawer.is_some() && a.drawer_dirty && !a.drawer_hover && since_drawer > 1500.0 {
            LAST_DRAWER.with(|l| *l.borrow_mut() = t);
            a.drawer_dirty = false;
            views::render_drawer(a);
        }
        let since_hud = t - LAST_HUD.with(|l| *l.borrow());
        if (a.hud_dirty && since_hud > 700.0) || since_hud > 5000.0 {
            LAST_HUD.with(|l| *l.borrow_mut() = t);
            a.hud_dirty = false;
            ui::render_hud(a);
            sync_hash(a);
        }
        let since_panel = t - LAST_PANEL.with(|l| *l.borrow());
        if a.panel_dirty && (since_panel > 1200.0 || !matches!(el("panel-body").and_then(|b| b.get_attribute("data-sel")), Some(ref s) if *s == format!("{:?}", sel_key(&a.selection)))) {
            LAST_PANEL.with(|l| *l.borrow_mut() = t);
            a.panel_dirty = false;
            rerender_panel(a);
        }
    });
}

#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    resize();
    load_levels();
    if let Some(p) = store_get(PINS_KEY) {
        with(|a| a.pins = p.lines().filter(|l| !l.is_empty()).map(str::to_string).collect());
    }
    if let Some(c) = store_get(COLOR_KEY) {
        set_color(ColorBy::from_id(&c));
    }
    if store_get(NOTIFY_KEY).as_deref() == Some("1") && notify::permission() == web_sys::NotificationPermission::Granted {
        with(|a| a.notify = true);
    }
    apply_hash(false);
    bind_ui();
    connect(1000);
    let ctx: CanvasRenderingContext2d = canvas().get_context("2d")?.unwrap().dyn_into()?;
    let f: Rc<RefCell<Option<Closure<dyn FnMut()>>>> = Rc::new(RefCell::new(None));
    let g = f.clone();
    *g.borrow_mut() = Some(Closure::new(move || {
        tick(&ctx);
        window().request_animation_frame(f.borrow().as_ref().unwrap().as_ref().unchecked_ref()).ok();
    }));
    window().request_animation_frame(g.borrow().as_ref().unwrap().as_ref().unchecked_ref())?;
    Ok(())
}
