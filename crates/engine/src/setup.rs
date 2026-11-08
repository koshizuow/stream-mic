//! Auto setup in the engine (build-plan.md step 8): the page starts it, the audio thread hands over the raw mic and
//! the computer's sound (Shared::rec), this thread takes 3 s of the room and 10 s of talking, runs
//! stream_mic_chain::autoset on them and puts the result into the settings (the current preset, as the user's own
//! changes to it, so "這個模式恢復預設" still goes back). "undo" puts back the preset from before.
//!
//! Also tells when the mic hears the computer's sound while the room should be quiet: the interface's loopback
//! (AG06's LOOPBACK, a "stereo mix") or speakers; with the computer's sound in the virtual mic that doubles it.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use stream_mic_chain::autoset::{self, Warning};
use stream_mic_chain::{Chain, Preset};

use crate::state::Shared;

const QUIET_S: f64 = 3.0;
const TALK_S: f64 = 10.0;

fn status(shared: &Shared, v: Value) {
    *shared.setup.lock().unwrap() = v;
}

/// Start (if not running already)
pub fn start(shared: Arc<Shared>) -> Result<(), String> {
    {
        let s = shared.setup.lock().unwrap();
        if matches!(s.get("phase").and_then(Value::as_str), Some("quiet" | "talk" | "working")) {
            return Ok(());
        }
    }
    shared.setup_cancel.store(false, Ordering::Release);
    status(&shared, json!({ "phase": "quiet", "left_s": QUIET_S }));
    std::thread::Builder::new()
        .name("autoset".into())
        .spawn(move || {
            let v = match run(&shared) {
                Ok(v) => v,
                Err(e) => json!({ "phase": "error", "error": e }),
            };
            shared.rec_on.store(false, Ordering::Release);
            status(&shared, v);
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn cancel(shared: &Shared) {
    shared.setup_cancel.store(true, Ordering::Release);
}

/// The preset from before the last auto setup, back
pub fn undo(shared: &Shared) -> Result<(), String> {
    let prev = shared.setup.lock().unwrap().get("before").cloned().ok_or("nothing to undo")?;
    crate::http::apply(shared, &json!({ "preset": prev }))?;
    status(shared, json!({ "phase": "undone" }));
    Ok(())
}

/// Take `secs` of (mic, computer sound) from the audio thread
fn record(shared: &Shared, phase: &str, secs: f64) -> Result<(Vec<f32>, Vec<f32>), String> {
    let n = (secs * crate::proc::RATE as f64) as usize;
    let (mut mic, mut pc) = (Vec::with_capacity(n), Vec::with_capacity(n));
    let mut buf = vec![0f32; 2 * 4800];
    let mut idle = 0;
    while mic.len() < n {
        if shared.setup_cancel.load(Ordering::Acquire) {
            return Err("cancelled".into());
        }
        let got = shared.rec.pop(&mut buf) / 2;
        if got == 0 {
            idle += 1;
            // no audio for 2 s: the mic stopped (unplugged, or the audio is starting again)
            if idle > 100 {
                return Err("沒有收到麥克風的聲音".into());
            }
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        idle = 0;
        for i in 0..got {
            mic.push(buf[2 * i]);
            pc.push(buf[2 * i + 1]);
        }
        let left = (n.saturating_sub(mic.len())) as f64 / crate::proc::RATE as f64;
        status(shared, json!({ "phase": phase, "left_s": (left * 10.0).round() / 10.0 }));
    }
    mic.truncate(n);
    pc.truncate(n);
    Ok((mic, pc))
}

fn run(shared: &Arc<Shared>) -> Result<Value, String> {
    // from now on the audio thread copies the mic here; anything older is dropped
    let mut sink = vec![0f32; 9600];
    while shared.rec.pop(&mut sink) > 0 {}
    shared.rec_on.store(true, Ordering::Release);
    let quiet = record(shared, "quiet", QUIET_S);
    let talk = quiet.as_ref().ok().map(|_| record(shared, "talk", TALK_S));
    shared.rec_on.store(false, Ordering::Release);
    let (room, room_pc) = match quiet {
        Ok(q) => q,
        Err(e) if e == "cancelled" => return Ok(json!({ "phase": "cancelled" })),
        Err(e) => return Err(e),
    };
    let (voice, _) = match talk {
        Some(Ok(t)) => t,
        Some(Err(e)) if e == "cancelled" => return Ok(json!({ "phase": "cancelled" })),
        Some(Err(e)) => return Err(e),
        None => unreachable!(),
    };
    status(shared, json!({ "phase": "working" }));

    // the mic hearing the computer's sound while the room was to be quiet
    let rms = |x: &[f32]| (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len().max(1) as f64).sqrt();
    let pc_on = rms(&room_pc) > 1e-3;
    let hears_pc = pc_on && crate::loopdet::bleed(&room, &room_pc, crate::proc::RATE as usize) >= 0.4;

    let before: Preset = shared.cfg.lock().unwrap().preset.clone();
    let mut chain = Chain::new(before.clone())?;
    let r = autoset::autoset(&mut chain, &before, &room, &voice, autoset::TARGET);
    let warnings: Vec<Value> = r
        .warnings
        .iter()
        .map(|w| match w {
            Warning::Noisy(d) => json!({ "kind": "noisy", "db": d }),
            Warning::TooQuiet(d) => json!({ "kind": "quiet", "db": d }),
            Warning::TooLoud(d) => json!({ "kind": "loud", "db": d }),
            Warning::NoVoice => json!({ "kind": "novoice" }),
        })
        .collect();
    let no_voice = r.warnings.contains(&Warning::NoVoice);
    if !no_voice {
        let p = serde_json::to_value(&r.preset).map_err(|e| e.to_string())?;
        crate::http::apply(shared, &json!({ "preset": p }))?;
    }
    Ok(json!({
        "phase": "done",
        "applied": !no_voice,
        "before": serde_json::to_value(&before).unwrap_or(Value::Null),
        "floor_db": r.floor_db,
        "voice_db": r.voice_db,
        "peak_db": r.peak_db,
        "gate": r.preset.exp.thr,
        "comp_thr": r.preset.comp.thr,
        "comp_gr_db": r.comp_gr_db,
        "makeup": r.preset.comp.makeup,
        "lufs": r.lufs,
        "st": r.st,
        "target": autoset::TARGET,
        "hears_pc": hears_pc,
        "warnings": warnings,
    }))
}
