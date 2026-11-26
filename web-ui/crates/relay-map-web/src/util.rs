use wasm_bindgen::JsCast;
use web_sys::{Document, Element, HtmlElement, HtmlInputElement, HtmlSelectElement, HtmlTextAreaElement, Window};

pub fn window() -> Window {
    web_sys::window().expect("window")
}
pub fn document() -> Document {
    window().document().expect("document")
}
pub fn el(id: &str) -> Option<Element> {
    document().get_element_by_id(id)
}
pub fn html(id: &str) -> Option<HtmlElement> {
    el(id).and_then(|e| e.dyn_into::<HtmlElement>().ok())
}
pub fn set_html(id: &str, s: &str) {
    if let Some(e) = el(id) {
        e.set_inner_html(s);
    }
}
pub fn show(id: &str, on: bool) {
    if let Some(e) = el(id) {
        let _ = if on { e.class_list().remove_1("hidden") } else { e.class_list().add_1("hidden") };
    }
}
pub fn input_value(id: &str) -> String {
    let Some(e) = el(id) else { return String::new() };
    if let Some(i) = e.dyn_ref::<HtmlInputElement>() {
        return i.value();
    }
    if let Some(t) = e.dyn_ref::<HtmlTextAreaElement>() {
        return t.value();
    }
    if let Some(s) = e.dyn_ref::<HtmlSelectElement>() {
        return s.value();
    }
    String::new()
}
pub fn set_input_value(id: &str, v: &str) {
    let Some(e) = el(id) else { return };
    if let Some(i) = e.dyn_ref::<HtmlInputElement>() {
        i.set_value(v);
    } else if let Some(t) = e.dyn_ref::<HtmlTextAreaElement>() {
        t.set_value(v);
    } else if let Some(s) = e.dyn_ref::<HtmlSelectElement>() {
        s.set_value(v);
    }
}
pub fn checked(id: &str) -> bool {
    el(id).and_then(|e| e.dyn_into::<HtmlInputElement>().ok()).map_or(false, |i| i.checked())
}

pub fn now_ms() -> f64 {
    js_sys::Date::now()
}

pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            _ => o.push(c),
        }
    }
    o
}

pub fn ago(ms: i64) -> String {
    if ms <= 0 {
        return "never".into();
    }
    let d = ((now_ms() as i64 - ms) / 1000).max(0);
    match d {
        0..=59 => format!("{d}s ago"),
        60..=3599 => format!("{}m ago", d / 60),
        3600..=86399 => format!("{}h {}m ago", d / 3600, (d % 3600) / 60),
        _ => format!("{}d ago", d / 86400),
    }
}

pub fn clock(ms: i64) -> String {
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ms as f64));
    format!("{:02}:{:02}:{:02}", d.get_hours(), d.get_minutes(), d.get_seconds())
}

pub fn day_clock(ms: i64) -> String {
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ms as f64));
    let days = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    format!("{} {:02}:{:02}", days[d.get_day() as usize % 7], d.get_hours(), d.get_minutes())
}

pub fn big(n: u64) -> String {
    let f = n as f64;
    if f >= 1e9 {
        format!("{:.2}B", f / 1e9)
    } else if f >= 1e6 {
        format!("{:.1}M", f / 1e6)
    } else if f >= 1e3 {
        format!("{:.1}k", f / 1e3)
    } else {
        n.to_string()
    }
}

pub fn bytes(n: u64) -> String {
    let f = n as f64;
    if f >= 1e6 {
        format!("{:.1} MB", f / 1e6)
    } else if f >= 1e3 {
        format!("{:.1} KB", f / 1e3)
    } else {
        format!("{n} B")
    }
}

pub fn hash(s: &str) -> u32 {
    let mut h: u32 = 2166136261;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    h
}

/// Shorten a path to its last `n` components.
pub fn short_path(p: &str, n: usize) -> String {
    let parts: Vec<&str> = p.split('/').filter(|x| !x.is_empty()).collect();
    if parts.len() <= n {
        return p.to_string();
    }
    format!("…/{}", parts[parts.len() - n..].join("/"))
}


#[derive(Clone, Copy)]
pub struct Rgb(pub f64, pub f64, pub f64);

pub fn parse_hex(c: &str) -> Rgb {
    let c = c.trim_start_matches('#');
    let v = u32::from_str_radix(c, 16).unwrap_or(0x9aa7ff);
    Rgb(((v >> 16) & 255) as f64, ((v >> 8) & 255) as f64, (v & 255) as f64)
}

pub fn rgba(c: Rgb, a: f64) -> String {
    format!("rgba({},{},{},{:.3})", c.0 as u8, c.1 as u8, c.2 as u8, a.clamp(0.0, 1.0))
}
