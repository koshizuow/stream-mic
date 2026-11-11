//! The local control (127.0.0.1 only): the page, its state, and meters as server-sent events.
//!
//!   GET  /             the settings page in the layout chosen (ui/a.html or b.html, built in with the rest of ui/;
//!                      STREAM_MIC_UI=folder serves them from there)
//!   GET  /api/state    the settings (state::Cfg) as JSON, plus "bypass", "builtins" (the presets to choose from) and
//!                      "platform" (linux, windows)
//!   POST /api/state    change some of them, any part: {"preset": {"exp": {"thr": -50}}} changes one value.
//!                      Also "preset_name" (switch to a built-in preset, with the user's changes to it if any),
//!                      "reset_preset": true (drop them), "knobs" ({"gate": -48, ...}, through the preset's knob
//!                      mapping), "bypass". A new "mic" or "out" starts the audio again. Saved to settings.json.
//!                      Not saved there: "autostart" (true/false: start with the computer), "show": true (the app
//!                      brings its window up). GET adds "app" (inside the app window), "autostart", "autostart_ok".
//!   GET  /api/stats    counters (underruns, drops, worst block, quantum, latencies and their parts, CPU time,
//!                      samples the send guard bent)
//!   GET  /api/autoset  auto setup's progress: {"phase": idle|quiet|talk|working|done|error|cancelled|undone,
//!                      "left_s", and when done what it found and set (setup.rs)}
//!   POST /api/autoset  {"start": true}, {"cancel": true}, {"undo": true} (the preset from before it)
//!   GET  /events       "meters" about 30 times a second, "devices" at the start and when they change
//!
//! Plain std::net and a thread per connection: there is one user, on the same machine.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use stream_mic_chain::Preset;

use crate::state::{Cfg, Shared};

/// The page's files, built in: two layouts sharing one stylesheet and one script, and the fonts (Inter, JetBrains
/// Mono, Noto Sans TC cut to the page's characters; tools/make_fonts.py). STREAM_MIC_UI=folder serves them from there
const FILES: &[(&str, &str, &[u8])] = &[
    ("a.html", "text/html; charset=utf-8", include_bytes!("../../../ui/a.html")),
    ("b.html", "text/html; charset=utf-8", include_bytes!("../../../ui/b.html")),
    ("common.css", "text/css; charset=utf-8", include_bytes!("../../../ui/common.css")),
    ("app.js", "text/javascript; charset=utf-8", include_bytes!("../../../ui/app.js")),
    ("fonts/inter.woff2", "font/woff2", include_bytes!("../../../ui/fonts/inter.woff2")),
    ("fonts/jetbrains-mono.woff2", "font/woff2", include_bytes!("../../../ui/fonts/jetbrains-mono.woff2")),
    ("fonts/noto-sans-tc.woff2", "font/woff2", include_bytes!("../../../ui/fonts/noto-sans-tc.woff2")),
];

/// A file of the page: (type, bytes)
fn file(name: &str) -> Option<(&'static str, Vec<u8>)> {
    let (_, ty, data) = FILES.iter().find(|(n, _, _)| *n == name)?;
    match std::env::var_os("STREAM_MIC_UI") {
        Some(dir) => std::fs::read(std::path::Path::new(&dir).join(name)).ok().map(|b| (*ty, b)),
        None => Some((*ty, data.to_vec())),
    }
}

pub fn start(shared: Arc<Shared>, port: u16) -> std::io::Result<u16> {
    let l = TcpListener::bind(("127.0.0.1", port))?;
    let port = l.local_addr()?.port();
    std::thread::Builder::new().name("http".into()).spawn(move || {
        for c in l.incoming().flatten() {
            let s = shared.clone();
            let _ = std::thread::Builder::new().name("http-conn".into()).spawn(move || {
                let _ = serve(c, &s);
            });
        }
    })?;
    Ok(port)
}

fn reply(c: &mut TcpStream, code: &str, ctype: &str, body: &[u8]) -> std::io::Result<()> {
    write!(c, "HTTP/1.1 {code}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", body.len())?;
    c.write_all(body)
}

fn serve(mut c: TcpStream, shared: &Arc<Shared>) -> std::io::Result<()> {
    c.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut r = BufReader::new(c.try_clone()?);
    let mut line = String::new();
    r.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
    let path = path.split('?').next().unwrap_or("/").to_string();
    let method = method.to_string();
    let mut len = 0usize;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 || h == "\r\n" || h == "\n" {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                len = v.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0u8; len.min(1 << 20)];
    r.read_exact(&mut body)?;
    match (method.as_str(), path.as_str()) {
        // the layout chosen in the settings
        ("GET", "/") => {
            let l = if shared.cfg.lock().unwrap().ui.layout == "b" { "b.html" } else { "a.html" };
            let (ty, b) = file(l).unwrap_or(("text/plain", b"page missing".to_vec()));
            reply(&mut c, "200 OK", ty, &b)
        }
        ("GET", "/api/state") => reply(&mut c, "200 OK", "application/json", state(shared).to_string().as_bytes()),
        ("POST", "/api/state") => match serde_json::from_slice::<Value>(&body) {
            Ok(v) => match apply(shared, &v) {
                Ok(()) => reply(&mut c, "200 OK", "application/json", state(shared).to_string().as_bytes()),
                Err(e) => reply(&mut c, "400 Bad Request", "text/plain; charset=utf-8", e.as_bytes()),
            },
            Err(e) => reply(&mut c, "400 Bad Request", "text/plain; charset=utf-8", e.to_string().as_bytes()),
        },
        ("GET", "/api/autoset") => reply(&mut c, "200 OK", "application/json", shared.setup.lock().unwrap().to_string().as_bytes()),
        ("POST", "/api/autoset") => {
            let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            let r = if v.get("start").and_then(Value::as_bool) == Some(true) {
                crate::setup::start(shared.clone())
            } else if v.get("cancel").and_then(Value::as_bool) == Some(true) {
                crate::setup::cancel(shared);
                Ok(())
            } else if v.get("undo").and_then(Value::as_bool) == Some(true) {
                crate::setup::undo(shared)
            } else {
                Err("start, cancel or undo".into())
            };
            match r {
                Ok(()) => reply(&mut c, "200 OK", "application/json", shared.setup.lock().unwrap().to_string().as_bytes()),
                Err(e) => reply(&mut c, "400 Bad Request", "text/plain; charset=utf-8", e.as_bytes()),
            }
        }
        ("GET", "/api/stats") => reply(&mut c, "200 OK", "application/json", stats(shared).to_string().as_bytes()),
        ("GET", "/events") => events(c, shared),
        ("GET", p) => match file(p.trim_start_matches('/')) {
            Some((ty, b)) => reply(&mut c, "200 OK", ty, &b),
            None => reply(&mut c, "404 Not Found", "text/plain", b"not found"),
        },
        _ => reply(&mut c, "404 Not Found", "text/plain", b"not found"),
    }
}

/// The settings as the page sees them
pub fn state(shared: &Shared) -> Value {
    let c = shared.cfg.lock().unwrap();
    let mut v = serde_json::to_value(&*c).unwrap_or(Value::Null);
    if let Some(o) = v.as_object_mut() {
        o.insert("bypass".into(), json!(c.bypass));
        // linux or windows: the page names the virtual mic and the computer sound's way in by it
        o.insert("platform".into(), json!(std::env::consts::OS));
        // Linux: the sink to send games and music to, as programs list it
        o.insert("sink_desc".into(), json!(crate::proc::sink_desc()));
        o.insert("app".into(), json!(shared.in_app.load(Ordering::Relaxed)));
        o.insert("autostart".into(), json!(crate::autostart::enabled()));
        o.insert("autostart_ok".into(), json!(crate::autostart::supported()));
        o.insert(
            "builtins".into(),
            Value::Array(
                stream_mic_chain::preset::BUILT_IN
                    .iter()
                    .filter_map(|(n, _)| Preset::built_in(n))
                    .map(|p| json!({ "name": p.name, "label": p.label, "about": p.about, "edited": c.edited.contains_key(&p.name) }))
                    .collect(),
            ),
        );
    }
    v
}

/// `b` into `a`: objects key by key (only keys `a` has), anything else replaced
fn merge(a: &mut Value, b: &Value) {
    match (a, b) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, x) in b {
                if let Some(y) = a.get_mut(k) {
                    merge(y, x);
                }
            }
        }
        (a, b) => *a = b.clone(),
    }
}

/// A change from the page: a built-in preset by name (or back to its defaults), knobs, then any settings given
pub fn apply(shared: &Shared, v: &Value) -> Result<(), String> {
    // not settings: the app's window, and start with the computer (the system keeps that one)
    if v.get("show").and_then(Value::as_bool) == Some(true) {
        shared.show.fetch_add(1, Ordering::Release);
    }
    if let Some(b) = v.get("autostart").and_then(Value::as_bool) {
        crate::autostart::set(b)?;
    }
    let mut err = None;
    let mut saved = None;
    shared.set_cfg(|cfg| {
        let before = cfg.clone();
        if let Some(n) = v.get("preset_name").and_then(Value::as_str) {
            match cfg.edited.get(n).cloned().or_else(|| Preset::built_in(n)) {
                Some(p) => cfg.preset = p,
                None => err = Some(format!("no preset {n}")),
            }
        }
        if v.get("reset_preset").and_then(Value::as_bool) == Some(true) {
            let n = cfg.preset.name.clone();
            cfg.edited.remove(&n);
            if let Some(p) = Preset::built_in(&n) {
                cfg.preset = p;
            }
        }
        if let Some(k) = v.get("knobs").and_then(Value::as_object) {
            for (name, x) in k {
                if let Some(x) = x.as_f64() {
                    cfg.preset.set_knob(name, x);
                }
            }
        }
        if let Some(b) = v.get("bypass").and_then(Value::as_bool) {
            cfg.bypass = b;
        }
        // the rest: merged into the settings, part by part
        let mut cur = serde_json::to_value(&*cfg).unwrap_or(Value::Null);
        if let (Some(o), Some(m)) = (cur.as_object_mut(), v.as_object()) {
            for (k, x) in m {
                if !matches!(k.as_str(), "preset_name" | "reset_preset" | "knobs" | "bypass" | "edited" | "builtins") {
                    if let Some(y) = o.get_mut(k) {
                        merge(y, x);
                    }
                }
            }
        }
        match serde_json::from_value::<Cfg>(cur) {
            Ok(mut c) => {
                c.bypass = cfg.bypass;
                // the gate knob is the expander's threshold itself: set either way, both agree
                if c.preset.exp.thr != before.preset.exp.thr && c.preset.knobs.gate == before.preset.knobs.gate {
                    c.preset.knobs.gate = c.preset.exp.thr.clamp(-80.0, -20.0);
                }
                // a reverb style chosen: its values; a value of the reverb changed by hand: no style any more
                let (bv, cv) = (&before.preset.verb, &c.preset.verb);
                if cv.style != bv.style && !cv.style.is_empty() {
                    let st = c.preset.verb.style.clone();
                    if !c.preset.set_verb_style(&st) {
                        err = Some(format!("no reverb style {st}"));
                        c.preset.verb.style = bv.style.clone();
                    }
                } else if (cv.size, cv.pre, cv.lowcut, cv.damp, cv.er, cv.er_ms, cv.hc) != (bv.size, bv.pre, bv.lowcut, bv.damp, bv.er, bv.er_ms, bv.hc) {
                    c.preset.verb.style = String::new();
                }
                // a compressor style chosen: its values; how it compresses or moves changed by hand: no style any more
                let (bc, cc) = (&before.preset.comp, &c.preset.comp);
                if cc.style != bc.style && !cc.style.is_empty() {
                    let st = c.preset.comp.style.clone();
                    if !c.preset.set_comp_style(&st) {
                        err = Some(format!("no compressor style {st}"));
                        c.preset.comp.style = bc.style.clone();
                    }
                } else if cc.two_knob && (cc.amount, cc.limit) != (bc.amount, bc.limit) {
                    // the two-knob mode: Amount or Compress/Limit brings the model's times along, and Makeup when it
                    // follows (unless Makeup itself came with it)
                    let same_makeup = cc.makeup == bc.makeup;
                    c.preset.comp.two_knob_times();
                    if same_makeup {
                        let b = before.preset.comp.clone();
                        c.preset.comp.follow_makeup(&b);
                    }
                } else if (cc.ratio, cc.knee, cc.att, cc.rel, cc.rel2, cc.slow, cc.memory, cc.auto_att) != (bc.ratio, bc.knee, bc.att, bc.rel, bc.rel2, bc.slow, bc.memory, bc.auto_att) {
                    c.preset.comp.style = String::new();
                }
                // the mode switched off by hand: not its style any more
                if !c.preset.comp.two_knob && c.preset.comp.style == "two-knob" {
                    c.preset.comp.style = String::new();
                }
                // an EQ style chosen: its values; an EQ value changed by hand (not by the Body knob): no style any more
                let (be, ce) = (&before.preset.eq, &c.preset.eq);
                let knob_moved = c.preset.knobs.body != before.preset.knobs.body;
                if ce.style != be.style && !ce.style.is_empty() {
                    let st = c.preset.eq.style.clone();
                    if !c.preset.set_eq_style(&st) {
                        err = Some(format!("no EQ style {st}"));
                        c.preset.eq.style = be.style.clone();
                    }
                } else if !knob_moved && (ce.low, ce.mid, ce.mid_f, ce.mid_q, ce.pres, ce.pres_f, ce.pres_q, ce.air, ce.hc) != (be.low, be.mid, be.mid_f, be.mid_q, be.pres, be.pres_f, be.pres_q, be.air, be.hc) {
                    c.preset.eq.style = String::new();
                }
                // the low cut set by hand: its base moves with it (the Body knob's thinning stays on top)
                if !knob_moved && c.preset.lowcut.freq != before.preset.lowcut.freq {
                    let b = c.preset.knobs.body;
                    c.preset.lowcut.base = c.preset.lowcut.freq - if b < 0.0 { -b * 0.5 } else { 0.0 };
                }
                // the preset name is not the page's to change
                c.preset.name = cfg.preset.name.clone();
                *cfg = c;
            }
            Err(e) => err = Some(e.to_string()),
        }
        // the user's changes to this preset, kept for when they come back to it
        let name = cfg.preset.name.clone();
        if Preset::built_in(&name).as_ref() == Some(&cfg.preset) {
            cfg.edited.remove(&name);
        } else {
            cfg.edited.insert(name, cfg.preset.clone());
        }
        if cfg.mic != before.mic || cfg.out != before.out {
            shared.restart.store(true, Ordering::Release);
        }
        saved = Some(cfg.clone());
    });
    if let (true, Some(c)) = (shared.persist, saved) {
        if let Err(e) = c.save() {
            eprintln!("could not save the settings: {e}");
        }
    }
    err.map_or(Ok(()), Err)
}

pub fn stats(shared: &Shared) -> Value {
    let c = &shared.count;
    let ms = |a: &std::sync::atomic::AtomicU64| a.load(Ordering::Relaxed) as f64 * 1000.0 / crate::proc::RATE as f64;
    json!({
        "send_under": c.send_under.load(Ordering::Relaxed),
        "mon_under": c.mon_under.load(Ordering::Relaxed),
        "send_drop": c.send_drop.load(Ordering::Relaxed),
        "mon_drop": c.mon_drop.load(Ordering::Relaxed),
        "blocks": c.blocks.load(Ordering::Relaxed),
        "worst_us": c.worst_us.load(Ordering::Relaxed),
        "busy_us": c.busy_us.load(Ordering::Relaxed),
        "send_clip": c.send_clip.load(Ordering::Relaxed),
        "quantum": c.quantum.load(Ordering::Relaxed),
        // the chain's own delay, ms, and its parts; the page adds the buffers (a quantum in and one out)
        "send_latency_ms": ms(&c.send_lat),
        "monitor_latency_ms": ms(&c.mon_lat),
        "send_denoise_ms": ms(&c.send_dn),
        "send_limiter_ms": ms(&c.send_lim),
        "monitor_denoise_ms": ms(&c.mon_dn),
        "monitor_limiter_ms": ms(&c.mon_lim),
        "denoise_ms": ms(&c.dn_lat),
        "dn_under": c.dn_under.load(Ordering::Relaxed),
        "dn_worst_us": c.dn_worst_us.load(Ordering::Relaxed),
        "dn_slow": c.dn_slow.load(Ordering::Relaxed),
        "pc_under": c.pc_under.load(Ordering::Relaxed),
        "pc_drop": c.pc_drop.load(Ordering::Relaxed),
        // Windows: each clock against the mic's, parts per million off (0 on Linux: one graph)
        "send_ppm": c.ratio_ppm[0].load(Ordering::Relaxed) as f64 - 1e6,
        "monitor_ppm": c.ratio_ppm[1].load(Ordering::Relaxed) as f64 - 1e6,
        "pc_ppm": c.ratio_ppm[2].load(Ordering::Relaxed) as f64 - 1e6,
        "loop_mute": shared.loop_mute.load(Ordering::Relaxed),
        "loop_corr": shared.loop_corr.load(Ordering::Relaxed) as f64 / 1000.0,
        "loop_lag_ms": shared.loop_lag_ms.load(Ordering::Relaxed),
    })
}

fn dbfs(x: f64) -> f64 {
    if x <= 1e-10 {
        -120.0
    } else {
        (20.0 * x.log10()).max(-120.0)
    }
}

fn events(mut c: TcpStream, shared: &Shared) -> std::io::Result<()> {
    write!(c, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: keep-alive\r\n\r\n")?;
    let mut devices = Value::Null;
    // loudness over the last 3 s (short-term, BS.1770), and the mic's level as the gate sees it over the last 10 s
    let mut kq: VecDeque<(f64, u64)> = VecDeque::new();
    // the same for the computer's sound as sent
    let mut pq: VecDeque<(f64, u64)> = VecDeque::new();
    // the scope: from 2 s back at first, then each point once
    let mut hist_at = shared.hist_buf.lock().unwrap().next.saturating_sub(400);
    let mut gq: VecDeque<f64> = VecDeque::new();
    let mut tick = 0u32;
    loop {
        if tick % 30 == 0 {
            let d = shared.devices.lock().unwrap().clone();
            if d != devices {
                write!(c, "event: devices\ndata: {d}\n\n")?;
                devices = d;
            }
        }
        tick += 1;
        std::thread::sleep(Duration::from_millis(33));
        let a = shared.take_acc();
        let n = a.n.max(1) as f64;
        kq.push_back((a.k_ms, a.n));
        while kq.len() > 90 {
            kq.pop_front();
        }
        pq.push_back((a.pcs_k_ms, a.n));
        while pq.len() > 90 {
            pq.pop_front();
        }
        if a.n > 0 {
            gq.push_back(a.gate_db / n);
            while gq.len() > 300 {
                gq.pop_front();
            }
        }
        let (ks, kn) = kq.iter().fold((0.0, 0u64), |(s, k), &(x, m)| (s + x, k + m));
        // the voice goes out on both channels: two channels of it, as OBS's meter would count
        let lufs = if kn > 0 && ks > 0.0 { -0.691 + 10.0 * (2.0 * ks / kn as f64).log10() } else { -120.0 };
        let (ps, pn) = pq.iter().fold((0.0, 0u64), |(s, k), &(x, m)| (s + x, k + m));
        let pc_lufs = if pn > 0 && ps > 0.0 { -0.691 + 10.0 * (ps / pn as f64).log10() } else { -120.0 };
        let hist = {
            let mut h = shared.hist_buf.lock().unwrap();
            h.drain(&shared.hist);
            let (pts, next) = h.since(hist_at);
            hist_at = next;
            pts
        };
        // rounded to 0.1 dB: the page draws them, it does not need more
        let r1 = |v: f32| (v as f64 * 10.0).round() / 10.0;
        // the floor: the quietest tenth of the last 10 s (the pauses)
        let floor = if gq.len() >= 30 {
            let mut s: Vec<f64> = gq.iter().copied().collect();
            s.sort_by(f64::total_cmp);
            s[s.len() / 10]
        } else {
            -120.0
        };
        let m = json!({
            "in_pk": dbfs(a.in_pk as f64), "in_rms": dbfs((a.in_ms / n).sqrt()),
            "out_pk": dbfs(a.out_pk as f64), "out_rms": dbfs((a.out_ms / n).sqrt()),
            "music": dbfs((a.pc_ms / n).sqrt()),
            // turned down now: the compressor and the limiter (loud parts), and the gate (pauses) apart
            "gr": -(a.gr_comp + a.gr_lim), "gr_gate": -a.gr_exp, "gr_comp": -a.gr_comp, "gr_lim": -a.gr_lim,
            // the computer's sound as it goes out with the voice (after its volume): peak and loudness over 3 s
            "pc_pk": dbfs(a.pcs_pk as f64), "pc_rms": dbfs((a.pcs_ms / n).sqrt()), "pc_lufs": pc_lufs.max(-120.0),
            // the scope: (mic peak, voice peak, reduction) every 5 ms since the last event
            "hist": hist.iter().map(|p| [r1(p[0]), r1(p[1]), r1(p[2])]).collect::<Vec<_>>(),
            "lufs": lufs.max(-120.0),
            "floor": floor,
            "frames": a.n,
            // the loopback muted: OBS or Discord is playing stream-mic back into the computer sound
            "loop": shared.loop_mute.load(Ordering::Relaxed),
        });
        write!(c, "event: meters\ndata: {m}\n\n")?;
        c.flush()?;
    }
}
