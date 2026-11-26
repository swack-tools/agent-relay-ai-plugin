//! Canvas rendering: background, pool regions, channels, pulses, nodes.

use crate::app::*;
use crate::insight::Health;
use crate::util::*;
use relay_map_core::*;
use std::f64::consts::{PI, TAU};
use web_sys::CanvasRenderingContext2d as Ctx;

fn bezier_ctrl(ax: f64, ay: f64, bx: f64, by: f64, bend: f64) -> (f64, f64) {
    let (mx, my) = ((ax + bx) / 2.0, (ay + by) / 2.0);
    let (dx, dy) = (bx - ax, by - ay);
    (mx - dy * bend, my + dx * bend)
}

pub fn bez(ax: f64, ay: f64, cx: f64, cy: f64, bx: f64, by: f64, t: f64) -> (f64, f64) {
    let u = 1.0 - t;
    (u * u * ax + 2.0 * u * t * cx + t * t * bx, u * u * ay + 2.0 * u * t * cy + t * t * by)
}

/// Screen-space endpoints and control point of the arc between two nodes.
pub fn arc_points(app: &App, a: &str, b: &str) -> Option<(f64, f64, f64, f64, f64, f64)> {
    let (na, nb) = (app.nodes.get(a)?, app.nodes.get(b)?);
    let (ax, ay) = app.world_to_screen(na.x, na.y);
    let (bx, by) = app.world_to_screen(nb.x, nb.y);
    // Bend direction is stable per pair so A->B and B->A share one arc.
    let (cx, cy) = if a < b { bezier_ctrl(ax, ay, bx, by, 0.18) } else { bezier_ctrl(bx, by, ax, ay, 0.18) };
    Some((ax, ay, cx, cy, bx, by))
}

pub fn frame(app: &mut App, ctx: &Ctx, t: f64) {
    let (w, h) = (app.width, app.height);
    ctx.set_transform(app.dpr, 0.0, 0.0, app.dpr, 0.0, 0.0).ok();
    background(app, ctx, w, h, t);
    regions(app, ctx, t);
    let focus = focus_set(app);
    channels(app, ctx, t, &focus);
    pulses(app, ctx, t);
    nodes(app, ctx, t, &focus);
    if let Selection::Agent(addr) = app.selection.clone() {
        orbit(app, ctx, t, &addr);
    }
}

/// Nodes to keep bright while something is selected.
fn focus_set(app: &App) -> Option<std::collections::HashSet<String>> {
    match &app.selection {
        Selection::None => None,
        Selection::Agent(a) => {
            let me = app.drawn_as.get(a).cloned().unwrap_or(a.clone());
            let mut s = std::collections::HashSet::new();
            s.insert(me.clone());
            for c in &app.channels {
                if c.a == me {
                    s.insert(c.b.clone());
                }
                if c.b == me {
                    s.insert(c.a.clone());
                }
            }
            Some(s)
        }
        Selection::Channel(a, b) => Some([a.clone(), b.clone()].into_iter().collect()),
    }
}

fn background(app: &App, ctx: &Ctx, w: f64, h: f64, t: f64) {
    let g = ctx.create_radial_gradient(w / 2.0, h / 2.0, 0.0, w / 2.0, h / 2.0, w.max(h) * 0.75).unwrap();
    g.add_color_stop(0.0, "#0b1430").ok();
    g.add_color_stop(1.0, "#03060f").ok();
    ctx.set_fill_style_canvas_gradient(&g);
    ctx.fill_rect(0.0, 0.0, w, h);
    // Dot matrix in world space, like the land mass on an attack map.
    let step = 34.0;
    let z = app.cam.zoom;
    if z * step > 9.0 {
        let (x0, y0) = app.screen_to_world(0.0, 0.0);
        let (x1, y1) = app.screen_to_world(w, h);
        let sx = (x0 / step).floor() as i64;
        let sy = (y0 / step).floor() as i64;
        ctx.set_fill_style_str("rgba(90,130,220,0.16)");
        let mut count = 0;
        for gy in sy..=((y1 / step).ceil() as i64) {
            for gx in sx..=((x1 / step).ceil() as i64) {
                count += 1;
                if count > 9000 {
                    break;
                }
                let wx = gx as f64 * step;
                let wy = gy as f64 * step;
                // Thin the grid away from the center for a "continent" feel.
                let d = (wx * wx + wy * wy).sqrt();
                let n = (((gx.wrapping_mul(73856093) ^ gy.wrapping_mul(19349663)) as u64 % 100) as f64) / 100.0;
                if n < (d / 1800.0).min(0.85) {
                    continue;
                }
                let (px, py) = app.world_to_screen(wx, wy);
                ctx.fill_rect(px - 0.8, py - 0.8, 1.6, 1.6);
            }
        }
    }
    // Rings and a radar sweep around the operator.
    let (cx, cy) = app.world_to_screen(0.0, 0.0);
    ctx.set_line_width(1.0);
    for i in 1..6 {
        let r = i as f64 * 160.0 * z;
        ctx.set_stroke_style_str(&format!("rgba(80,140,255,{:.3})", 0.07 - i as f64 * 0.01));
        ctx.begin_path();
        ctx.arc(cx, cy, r, 0.0, TAU).ok();
        ctx.stroke();
    }
    let sweep = (t / 4000.0) % 1.0 * TAU;
    let rr = 900.0 * z;
    for k in 0..24 {
        let a0 = sweep - k as f64 * 0.025;
        ctx.set_fill_style_str(&format!("rgba(70,160,255,{:.3})", 0.028 * (1.0 - k as f64 / 24.0)));
        ctx.begin_path();
        ctx.move_to(cx, cy);
        ctx.arc(cx, cy, rr, a0 - 0.025, a0).ok();
        ctx.close_path();
        ctx.fill();
    }
}

fn regions(app: &mut App, ctx: &Ctx, t: f64) {
    for r in app.regions.iter_mut() {
        r.x += (r.tx - r.x) * 0.08;
        r.y += (r.ty - r.y) * 0.08;
        r.r += (r.tr - r.r) * 0.08;
    }
    // One band per computer behind its project pools.
    let (ox, oy) = app.world_to_screen(0.0, 0.0);
    for b in &app.host_bands {
        let col = parse_hex(&host_color(&b.host));
        let rad = b.radius * app.cam.zoom;
        let max_r = app.regions.iter().filter(|r| r.host.as_deref() == Some(b.host.as_str())).map(|r| r.r).fold(0.0, f64::max);
        let outer = rad + (max_r + 34.0) * app.cam.zoom;
        ctx.set_line_cap("round");
        ctx.set_stroke_style_str(&rgba(col, 0.10));
        ctx.set_line_width((2.0 * max_r + 40.0) * app.cam.zoom);
        ctx.begin_path();
        ctx.arc(ox, oy, rad, b.a0, b.a1).ok();
        ctx.stroke();
        ctx.set_stroke_style_str(&rgba(col, 0.55));
        ctx.set_line_width(2.0);
        ctx.begin_path();
        ctx.arc(ox, oy, outer, b.a0, b.a1).ok();
        ctx.stroke();
        let mid = (b.a0 + b.a1) / 2.0;
        let (lx, ly) = (ox + (outer + 16.0) * mid.cos(), oy + (outer + 16.0) * mid.sin());
        let fs = (15.0 * app.cam.zoom.sqrt()).clamp(11.0, 20.0);
        ctx.set_font(&format!("700 {fs:.0}px 'JetBrains Mono', ui-monospace, monospace"));
        ctx.set_text_align("center");
        ctx.set_text_baseline("middle");
        ctx.set_fill_style_str(&rgba(col, 0.95));
        ctx.fill_text(&format!("▣ {}", b.host.to_uppercase()), lx, ly).ok();
        ctx.set_text_baseline("alphabetic");
    }
    for r in &app.regions {
        let (x, y) = app.world_to_screen(r.x, r.y);
        let rad = r.r * app.cam.zoom;
        let deep = (r.depth.max(1) - 1) as f64;
        let g = ctx.create_radial_gradient(x, y, rad * 0.2, x, y, rad).unwrap();
        g.add_color_stop(0.0, &rgba(r.color, 0.09 / (1.0 + deep * 0.5))).ok();
        g.add_color_stop(1.0, &rgba(r.color, 0.03)).ok();
        ctx.set_fill_style_canvas_gradient(&g);
        ctx.begin_path();
        ctx.arc(x, y, rad, 0.0, TAU).ok();
        ctx.fill();
        ctx.set_line_width(if r.depth <= 1 { 1.6 } else { 1.0 });
        ctx.set_stroke_style_str(&rgba(r.color, 0.55 - deep * 0.1));
        let dash = js_sys::Array::of2(&6.0.into(), &8.0.into());
        ctx.set_line_dash(&dash).ok();
        ctx.set_line_dash_offset(-t / 120.0);
        ctx.stroke();
        ctx.set_line_dash(&js_sys::Array::new()).ok();
        // Label on the rim.
        let fs = ((14.0 - deep * 2.0) * app.cam.zoom.sqrt()).clamp(9.0, 18.0);
        // Inner labels only once there is room for them.
        if rad < 22.0 || (deep > 0.0 && rad < 40.0) {
            continue;
        }
        ctx.set_font(&format!("{} {fs:.0}px 'JetBrains Mono', ui-monospace, monospace", if deep == 0.0 { 700 } else { 500 }));
        ctx.set_text_align("center");
        ctx.set_fill_style_str(&rgba(r.color, 0.95 - deep * 0.15));
        let lock = if r.isolated { " ◆ isolated" } else { "" };
        let text = if deep == 0.0 { r.label.to_uppercase() } else { truncate(&r.label, 34) };
        let ly = if deep == 0.0 { y - rad - 10.0 } else { y - rad + fs + 4.0 };
        ctx.fill_text(&format!("{}  ·  {}{}", text, r.members, lock), x, ly).ok();
        // Agent-type sectors: a faint divider and a small label for each type.
        if !r.kinds.is_empty() && app.cam.zoom > 0.3 {
            let fs = (10.0 * app.cam.zoom.sqrt()).clamp(8.0, 13.0);
            ctx.set_font(&format!("{fs:.0}px 'JetBrains Mono', ui-monospace, monospace"));
            for (k, mid, n) in &r.kinds {
                let kc = match k.as_str() {
                    "claude" => kind_color(Some(AgentKind::Claude)),
                    "codex" => kind_color(Some(AgentKind::Codex)),
                    "antigravity" => kind_color(Some(AgentKind::Antigravity)),
                    "opencode" => kind_color(Some(AgentKind::Opencode)),
                    "pi" => kind_color(Some(AgentKind::Pi)),
                    _ => kind_color(None),
                };
                let lr = rad - 9.0;
                ctx.set_fill_style_str(&rgba(kc, 0.75));
                ctx.fill_text(&format!("{} {}", if k == "codex" { "openai" } else { k.as_str() }, n), x + lr * mid.cos(), y + lr * mid.sin() + 3.0).ok();
            }
        }
    }
}

fn channels(app: &App, ctx: &Ctx, t: f64, focus: &Option<std::collections::HashSet<String>>) {
    let now = now_ms() as i64;
    for c in &app.channels {
        let Some((ax, ay, cx, cy, bx, by)) = arc_points(app, &c.a, &c.b) else { continue };
        let lit = focus.as_ref().map_or(true, |f| f.contains(&c.a) && f.contains(&c.b));
        let sel = app.selection == Selection::Channel(c.a.clone(), c.b.clone()) || app.hover_channel.as_ref().map_or(false, |(x, y)| (x == &c.a && y == &c.b) || (x == &c.b && y == &c.a));
        let recent = ((now - c.last_ms) as f64 / 3_600_000.0).clamp(0.0, 1.0);
        let base_a = if c.swarm_only { 0.10 } else { 0.22 } + 0.25 * (1.0 - recent);
        let alpha = if sel { 0.95 } else if lit { base_a } else { 0.04 };
        let width = (0.8 + (c.count as f64).log2().max(0.0) * 0.8).min(6.0) * if sel { 1.8 } else { 1.0 };
        // Glow underlay.
        ctx.set_line_cap("round");
        ctx.set_stroke_style_str(&rgba(c.color, alpha * 0.35));
        ctx.set_line_width(width * 4.0);
        ctx.begin_path();
        ctx.move_to(ax, ay);
        ctx.quadratic_curve_to(cx, cy, bx, by);
        ctx.stroke();
        // Flowing core: dashes move in the dominant direction.
        ctx.set_stroke_style_str(&rgba(c.color, (alpha * 2.2).min(1.0)));
        ctx.set_line_width(width);
        let dash = js_sys::Array::of2(&3.0.into(), &9.0.into());
        ctx.set_line_dash(&dash).ok();
        let dir = if c.a_to_b * 2 >= c.count { -1.0 } else { 1.0 };
        ctx.set_line_dash_offset(dir * t / 40.0);
        ctx.begin_path();
        ctx.move_to(ax, ay);
        ctx.quadratic_curve_to(cx, cy, bx, by);
        ctx.stroke();
        ctx.set_line_dash(&js_sys::Array::new()).ok();
        if sel || (lit && focus.is_some()) {
            let (mx, my) = bez(ax, ay, cx, cy, bx, by, 0.5);
            ctx.set_font("11px 'JetBrains Mono', ui-monospace, monospace");
            ctx.set_text_align("center");
            ctx.set_fill_style_str("rgba(220,230,255,0.9)");
            ctx.fill_text(&format!("{} msgs · {}", c.count, bytes(c.bytes)), mx, my - 6.0).ok();
        }
    }
}

fn pulses(app: &mut App, ctx: &Ctx, t: f64) {
    let mut arrivals = Vec::new();
    app.pulses.retain(|p| t - p.t0 < p.dur + 50.0);
    for p in &app.pulses {
        let k = ((t - p.t0) / p.dur).clamp(0.0, 1.0);
        if p.internal {
            if let Some(n) = app.nodes.get(&p.to) {
                let (x, y) = app.world_to_screen(n.x, n.y);
                let rr = (n.r + 4.0 + k * (18.0 + p.size * 2.0)) * app.cam.zoom.max(0.4);
                ctx.set_stroke_style_str(&rgba(p.color, 0.6 * (1.0 - k)));
                ctx.set_line_width(1.5);
                ctx.begin_path();
                ctx.arc(x, y, rr, 0.0, TAU).ok();
                ctx.stroke();
            }
            continue;
        }
        let Some((ax, ay, cx, cy, bx, by)) = arc_points(app, &p.from, &p.to) else { continue };
        // Ease so the bolt leaves fast and settles.
        let e = 1.0 - (1.0 - k).powi(2);
        let zs = app.cam.zoom.sqrt().clamp(0.5, 1.6);
        let size = p.size * zs * if p.swarm { 0.6 } else if p.cross { 1.3 } else { 1.0 };
        // Beam for big payloads: a thick stroke along the trail.
        let tail = (e - 0.22 - size / 60.0).max(0.0);
        let steps = 14;
        for i in 0..steps {
            let u0 = tail + (e - tail) * i as f64 / steps as f64;
            let u1 = tail + (e - tail) * (i + 1) as f64 / steps as f64;
            let (x0, y0) = bez(ax, ay, cx, cy, bx, by, u0);
            let (x1, y1) = bez(ax, ay, cx, cy, bx, by, u1);
            let a = (i + 1) as f64 / steps as f64;
            ctx.set_stroke_style_str(&rgba(p.color, 0.55 * a));
            ctx.set_line_width((size * 0.9 * a).max(1.0));
            ctx.set_line_cap("round");
            ctx.begin_path();
            ctx.move_to(x0, y0);
            ctx.line_to(x1, y1);
            ctx.stroke();
        }
        let (hx, hy) = bez(ax, ay, cx, cy, bx, by, e);
        let g = ctx.create_radial_gradient(hx, hy, 0.0, hx, hy, size * 2.6).unwrap();
        g.add_color_stop(0.0, "rgba(255,255,255,0.95)").ok();
        g.add_color_stop(0.25, &rgba(p.color, 0.9)).ok();
        g.add_color_stop(1.0, &rgba(p.color, 0.0)).ok();
        ctx.set_fill_style_canvas_gradient(&g);
        ctx.begin_path();
        ctx.arc(hx, hy, size * 2.6, 0.0, TAU).ok();
        ctx.fill();
        if k >= 1.0 {
            arrivals.push((p.to.clone(), p.color, size));
        }
    }
    for (to, color, size) in arrivals {
        if let Some(n) = app.nodes.get(&to) {
            app.ripples.push(Ripple { x: n.x, y: n.y, t0: t, color, size });
        }
    }
    app.ripples.retain(|r| t - r.t0 < 900.0);
    for r in &app.ripples {
        let k = (t - r.t0) / 900.0;
        let (x, y) = app.world_to_screen(r.x, r.y);
        ctx.set_stroke_style_str(&rgba(r.color, 0.8 * (1.0 - k)));
        ctx.set_line_width(2.0 * (1.0 - k) + 0.5);
        ctx.begin_path();
        ctx.arc(x, y, (10.0 + k * (24.0 + r.size * 3.0)) * app.cam.zoom.sqrt().clamp(0.5, 1.6), 0.0, TAU).ok();
        ctx.stroke();
    }
    // Keep pulses in the hit list short.
    if app.pulses.len() > 400 {
        let n = app.pulses.len() - 400;
        app.pulses.drain(0..n);
    }
}

fn nodes(app: &mut App, ctx: &Ctx, t: f64, focus: &Option<std::collections::HashSet<String>>) {
    for n in app.nodes.values_mut() {
        n.x += (n.tx - n.x) * 0.09;
        n.y += (n.ty - n.y) * 0.09;
    }
    let now = now_ms() as i64;
    let mut order: Vec<&String> = app.nodes.keys().collect();
    order.sort();
    for addr in order {
        let n = &app.nodes[addr];
        let Some(a) = app.agents.get(addr) else { continue };
        let (x, y) = app.world_to_screen(n.x, n.y);
        if x < -60.0 || y < -60.0 || x > app.width + 60.0 || y > app.height + 60.0 {
            continue;
        }
        let z = app.cam.zoom.sqrt().clamp(0.45, 1.8);
        let r = n.r * z;
        let col = app.node_color(addr);
        let health = app.insight(addr).map(|i| i.health);
        let lit = focus.as_ref().map_or(true, |f| f.contains(addr)) && app.matches_search(a);
        let dim = if lit { 1.0 } else { 0.18 };
        let appear = ((now_ms() - n.born) / 600.0).clamp(0.0, 1.0);
        let recent = now - a.last_active_ms < 5 * 60 * 1000;
        let busy = a.state.as_deref() == Some("busy") || recent;
        // Halo for live agents.
        if (a.alive == Some(true) || recent) && !a.ghost {
            let pulse = 0.5 + 0.5 * ((t / 700.0) + (hash(addr) % 100) as f64).sin();
            let g = ctx.create_radial_gradient(x, y, r * 0.5, x, y, r * (2.2 + pulse * 0.6)).unwrap();
            g.add_color_stop(0.0, &rgba(col, 0.35 * dim)).ok();
            g.add_color_stop(1.0, &rgba(col, 0.0)).ok();
            ctx.set_fill_style_canvas_gradient(&g);
            ctx.begin_path();
            ctx.arc(x, y, r * 3.0, 0.0, TAU).ok();
            ctx.fill();
        }
        // Body.
        ctx.set_fill_style_str(&format!("rgba(8,14,30,{:.2})", 0.92 * dim.max(0.4)));
        ctx.begin_path();
        ctx.arc(x, y, r * appear, 0.0, TAU).ok();
        ctx.fill();
        ctx.set_line_width(if app.hover.as_deref() == Some(addr) { 3.0 } else { 2.0 });
        ctx.set_stroke_style_str(&rgba(col, 0.95 * dim));
        if a.ghost {
            ctx.set_line_dash(&js_sys::Array::of2(&2.0.into(), &3.0.into())).ok();
        }
        ctx.stroke();
        ctx.set_line_dash(&js_sys::Array::new()).ok();
        // Host tick on the rim.
        ctx.set_fill_style_str(&rgba(parse_hex(&host_color(&a.host)), 0.95 * dim));
        ctx.begin_path();
        ctx.arc(x + r * 0.72, y - r * 0.72, (r * 0.28).max(2.2), 0.0, TAU).ok();
        ctx.fill();
        // Inner glyph by kind.
        ctx.set_fill_style_str(&rgba(col, 0.9 * dim));
        ctx.set_font(&format!("700 {:.0}px 'JetBrains Mono', ui-monospace, monospace", (r * 0.95).max(8.0)));
        ctx.set_text_align("center");
        ctx.set_text_baseline("middle");
        let glyph = match a.kind {
            Some(AgentKind::Claude) => "C",
            Some(AgentKind::Codex) => "X",
            Some(AgentKind::Antigravity) => "A",
            Some(AgentKind::Opencode) => "O",
            Some(AgentKind::Pi) => "π",
            Some(AgentKind::Operator) => "◎",
            None => "?",
        };
        ctx.fill_text(glyph, x, y + 0.5).ok();
        ctx.set_text_baseline("alphabetic");
        // Busy spinner.
        if busy && !a.ghost && a.kind != Some(AgentKind::Operator) {
            let s = (t / 500.0) % TAU;
            ctx.set_stroke_style_str(&rgba(col, 0.9 * dim));
            ctx.set_line_width(1.5);
            ctx.begin_path();
            ctx.arc(x, y, r + 4.0, s, s + PI * 0.6).ok();
            ctx.stroke();
        }
        // Health badge: stuck and looping agents shout; waiting ones get a dashed amber ring.
        match health {
            Some(h @ (Health::Stuck | Health::Looping)) => {
                let hc = h.color();
                let beat = 0.55 + 0.45 * (t / 260.0).sin().abs();
                let (bx, by, br) = (x - r * 0.8, y - r * 0.8, (r * 0.42).max(5.5));
                ctx.set_stroke_style_str(&rgba(hc, 0.55 * beat * dim));
                ctx.set_line_width(2.0);
                ctx.begin_path();
                ctx.arc(x, y, r + 7.0 + 3.0 * beat, 0.0, TAU).ok();
                ctx.stroke();
                ctx.set_fill_style_str(&rgba(hc, 0.95 * dim));
                ctx.begin_path();
                ctx.arc(bx, by, br, 0.0, TAU).ok();
                ctx.fill();
                ctx.set_fill_style_str(&format!("rgba(10,10,20,{:.2})", dim));
                ctx.set_font(&format!("700 {:.0}px 'JetBrains Mono', ui-monospace, monospace", br * 1.5));
                ctx.set_text_align("center");
                ctx.set_text_baseline("middle");
                ctx.fill_text(if h == Health::Stuck { "!" } else { "↻" }, bx, by + 0.5).ok();
                ctx.set_text_baseline("alphabetic");
            }
            Some(Health::Waiting) => {
                ctx.set_stroke_style_str(&rgba(Health::Waiting.color(), 0.7 * dim));
                ctx.set_line_width(1.5);
                ctx.set_line_dash(&js_sys::Array::of2(&3.0.into(), &4.0.into())).ok();
                ctx.set_line_dash_offset(-t / 200.0);
                ctx.begin_path();
                ctx.arc(x, y, r + 6.0, 0.0, TAU).ok();
                ctx.stroke();
                ctx.set_line_dash(&js_sys::Array::new()).ok();
            }
            _ => {}
        }
        if app.pins.contains(addr) {
            ctx.set_fill_style_str(&format!("rgba(255,209,102,{:.2})", 0.95 * dim));
            ctx.set_font(&format!("{:.0}px 'JetBrains Mono', ui-monospace, monospace", (r * 0.9).max(10.0)));
            ctx.set_text_align("center");
            ctx.fill_text("★", x - r * 0.85, y + r * 1.15).ok();
        }
        // Folded swarm children as satellites.
        let kids = n.children.len();
        if kids > 0 && app.selection != Selection::Agent(addr.clone()) {
            let shown = kids.min(14);
            for i in 0..shown {
                let th = t / 3000.0 + i as f64 / shown as f64 * TAU;
                let rr = r + 9.0 * z;
                ctx.set_fill_style_str(&rgba(col, 0.7 * dim));
                ctx.begin_path();
                ctx.arc(x + rr * th.cos(), y + rr * th.sin(), 1.6 * z, 0.0, TAU).ok();
                ctx.fill();
            }
        }
        // Label.
        let talks = app.channels.iter().any(|c| (&c.a == addr || &c.b == addr) && !c.swarm_only);
        // Dimmed agents (outside a search or selection) keep quiet unless hovered.
        let show_label = (app.cam.zoom > 1.1 && lit)
            || (app.cam.zoom > 0.35 && lit && (talks || recent || kids > 0))
            || app.hover.as_deref() == Some(addr)
            || a.kind == Some(AgentKind::Operator)
            || focus.as_ref().map_or(false, |f| f.contains(addr))
            || (!app.query.is_empty() && lit)
            || app.pins.contains(addr)
            || matches!(health, Some(Health::Stuck | Health::Looping));
        if show_label {
            let name = app.names.get(addr).cloned().unwrap_or_else(|| a.display_name());
            let label = if kids > 0 { format!("{name} +{kids}") } else { name };
            ctx.set_font(&format!("{:.0}px 'JetBrains Mono', ui-monospace, monospace", (10.5 * z).clamp(9.0, 14.0)));
            ctx.set_fill_style_str(&format!("rgba(225,232,255,{:.2})", 0.9 * dim));
            ctx.fill_text(&truncate(&label, 26), x, y + r + 14.0 * z).ok();
        }
    }
}

/// Focused agent: sub-agents and swarm children circle it.
fn orbit(app: &App, ctx: &Ctx, t: f64, addr: &str) {
    let me = app.drawn_as.get(addr).cloned().unwrap_or(addr.to_string());
    let (Some(n), Some(a)) = (app.nodes.get(&me), app.agents.get(addr)) else { return };
    let (x, y) = app.world_to_screen(n.x, n.y);
    let mut sats: Vec<(String, String, bool)> = a
        .subagents
        .iter()
        .rev()
        .take(12)
        .map(|s| {
            let live = now_ms() as i64 - s.last_ms < 3 * 60 * 1000 && s.status != "done";
            (s.name.clone(), s.kind.clone(), live)
        })
        .collect();
    let mut kids: Vec<&String> = n.children.iter().collect();
    kids.sort_by_key(|c| -app.agents.get(*c).map_or(0, |a| a.last_active_ms));
    for c in kids.into_iter().take(12) {
        if let Some(ca) = app.agents.get(c) {
            let live = now_ms() as i64 - ca.last_active_ms < 5 * 60 * 1000;
            sats.push((ca.display_name(), "swarm".into(), live));
        }
    }
    if sats.is_empty() {
        return;
    }
    let col = kind_color(a.kind);
    let per = 9usize;
    let base = n.r * app.cam.zoom.sqrt().clamp(0.45, 1.8) + 70.0;
    let rings = (sats.len() as f64 / per as f64).ceil() as usize;
    for ring in 0..rings {
        let rr = base + ring as f64 * 62.0;
        ctx.set_stroke_style_str(&rgba(col, 0.18));
        ctx.set_line_width(1.0);
        ctx.begin_path();
        ctx.arc(x, y, rr, 0.0, TAU).ok();
        ctx.stroke();
    }
    let total = sats.len();
    for (i, (name, kind, live)) in sats.iter().enumerate() {
        let ring = i / per;
        let in_ring = (total - ring * per).min(per);
        let rr = base + ring as f64 * 62.0;
        let th = t / (14000.0 + ring as f64 * 5000.0) * if ring % 2 == 0 { 1.0 } else { -1.0 } + (i % per) as f64 / in_ring as f64 * TAU + ring as f64 * 0.35;
        let (sx, sy) = (x + rr * th.cos(), y + rr * th.sin());
        ctx.set_stroke_style_str(&rgba(col, 0.25));
        ctx.begin_path();
        ctx.move_to(x, y);
        ctx.line_to(sx, sy);
        ctx.stroke();
        let c2 = if kind == "swarm" { parse_hex("#7fd4ff") } else { col };
        ctx.set_fill_style_str(&rgba(c2, if *live { 0.95 } else { 0.45 }));
        ctx.begin_path();
        ctx.arc(sx, sy, if *live { 5.0 } else { 3.5 }, 0.0, TAU).ok();
        ctx.fill();
        ctx.set_font("10px 'JetBrains Mono', ui-monospace, monospace");
        ctx.set_text_align("center");
        ctx.set_fill_style_str(if *live { "rgba(235,240,255,0.95)" } else { "rgba(200,210,240,0.55)" });
        ctx.fill_text(&truncate(name, 22), sx, sy - 9.0).ok();
    }
}

/// Distance from a point to a quadratic arc, sampled.
pub fn dist_to_arc(px: f64, py: f64, p: (f64, f64, f64, f64, f64, f64)) -> f64 {
    let (ax, ay, cx, cy, bx, by) = p;
    let mut best = f64::MAX;
    let mut prev = (ax, ay);
    for i in 1..=24 {
        let cur = bez(ax, ay, cx, cy, bx, by, i as f64 / 24.0);
        let (dx, dy) = (cur.0 - prev.0, cur.1 - prev.1);
        let len2 = dx * dx + dy * dy;
        let u = if len2 > 0.0 { (((px - prev.0) * dx + (py - prev.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
        let (qx, qy) = (prev.0 + u * dx, prev.1 + u * dy);
        best = best.min(((px - qx).powi(2) + (py - qy).powi(2)).sqrt());
        prev = cur;
    }
    best
}
