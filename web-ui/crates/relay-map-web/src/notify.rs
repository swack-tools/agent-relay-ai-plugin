//! Desktop notifications (opt in). Clicking one focuses the agent.

use crate::util::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{Notification, NotificationOptions, NotificationPermission};

pub fn permission() -> NotificationPermission {
    Notification::permission()
}

/// Ask the browser for permission; `done(granted)` runs with the answer.
pub fn request(done: impl FnOnce(bool) + 'static) {
    if permission() == NotificationPermission::Granted {
        done(true);
        return;
    }
    let Ok(p) = Notification::request_permission() else {
        done(false);
        return;
    };
    let cb = Closure::once(move |v: JsValue| done(v.as_string().as_deref() == Some("granted")));
    let _ = p.then(&cb);
    cb.forget();
}

pub fn send(title: &str, body: &str, addr: &str) {
    if permission() != NotificationPermission::Granted {
        return;
    }
    let opts = NotificationOptions::new();
    opts.set_body(body);
    // One notification per agent: a newer one replaces the older.
    opts.set_tag(addr);
    let Ok(n) = Notification::new_with_options(title, &opts) else { return };
    let addr = addr.to_string();
    let onclick = Closure::<dyn FnMut()>::new(move || {
        let _ = window().focus();
        crate::focus_from_outside(&addr);
    });
    n.set_onclick(Some(onclick.as_ref().unchecked_ref()));
    onclick.forget();
}
