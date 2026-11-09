//! PipeWire (build-plan.md step 6, routing.md): four streams on one audio thread.
//!
//!   mic (a hardware source) ──► chain ──┬──► "stream-mic" (a virtual source: OBS, Discord)  + the computer's sound
//!                                       └──► the headphones (a hardware sink)                + the computer's sound
//!   "stream-mic 電腦聲音" (a virtual sink: games, music) ──► both of the above, and the reverb's music follower
//!
//! No feedback: the headphones go to a hardware sink named at start, never to the default (which may be our own
//! sink), and the mic is never our own source. All four callbacks run on PipeWire's one data thread (RT_PROCESS),
//! one after another, so they share their buffers through a RefCell without locks; the control side reaches them
//! only through `Shared` (try_lock, never waiting).
//!
//! The mic's callback runs the chain and fills two rings; the virtual source and the headphones take from them. A
//! ring is kept near one quantum: less and the reader gets silence (counted), more and the oldest go (counted).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use pipewire as pw;
use pw::properties::Properties;
use pw::spa;
use pw::spa::pod::Pod;
use crate::proc::{fill, ours, pick, Node, Opts, Rt, MAX_BLOCK, RATE};
use crate::state::{Ring, Shared};

pub const SOURCE_NAME: &str = "stream-mic";
pub const SINK_NAME: &str = "stream-mic-pc-audio";

/// The audio devices, for `--list`
pub fn list() -> Result<Vec<Node>, String> {
    pw::init();
    let ml = pw::main_loop::MainLoop::new(None).map_err(|e| e.to_string())?;
    let ctx = pw::context::Context::new(&ml).map_err(|e| e.to_string())?;
    let core = ctx.connect(None).map_err(|e| e.to_string())?;
    nodes(&core, &ml).map_err(|e| e.to_string())
}

/// The audio nodes there are now (half a second of listening to the registry)
pub fn nodes(core: &pw::core::Core, mainloop: &pw::main_loop::MainLoop) -> Result<Vec<Node>, pw::Error> {
    let found = Rc::new(RefCell::new(Vec::new()));
    let registry = core.get_registry()?;
    let f = found.clone();
    let _l = registry
        .add_listener_local()
        .global(move |g| {
            let Some(p) = g.props else { return };
            let class = p.get("media.class").unwrap_or("");
            // Audio/Source/Virtual too: a virtual mic another program made (a denoiser, a test source)
            if class.starts_with("Audio/Source") || class.starts_with("Audio/Sink") {
                f.borrow_mut().push(Node {
                    name: p.get("node.name").unwrap_or("").to_string(),
                    desc: p.get("node.description").or(p.get("node.nick")).unwrap_or("").to_string(),
                    class: class.to_string(),
                    prio: p.get("priority.session").and_then(|v| v.parse().ok()).unwrap_or(0),
                });
            }
        })
        .register();
    let ml = mainloop.clone();
    let timer = mainloop.loop_().add_timer(move |_| ml.quit());
    timer.update_timer(Some(Duration::from_millis(400)), None).into_result()?;
    mainloop.run();
    let v = found.borrow().clone();
    Ok(v)
}

/// EnumFormat: 32-bit float, 48 kHz, mono or stereo (FL FR), or 0: the device's own channels (with
/// stream.dont-remix, so PipeWire does not mix them down)
pub fn format(ch: u32) -> Vec<u8> {
    let mut info = spa::param::audio::AudioInfoRaw::new();
    info.set_format(spa::param::audio::AudioFormat::F32LE);
    info.set_rate(RATE);
    if ch > 0 {
        info.set_channels(ch);
        let mut pos = [0u32; 64];
        if ch == 1 {
            pos[0] = spa::sys::SPA_AUDIO_CHANNEL_MONO;
        } else {
            pos[0] = spa::sys::SPA_AUDIO_CHANNEL_FL;
            pos[1] = spa::sys::SPA_AUDIO_CHANNEL_FR;
        }
        info.set_position(pos);
    }
    let obj = spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: spa::param::ParamType::EnumFormat.as_raw(),
        properties: info.into(),
    };
    spa::pod::serialize::PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &spa::pod::Value::Object(obj))
        .expect("format pod")
        .0
        .into_inner()
}

pub fn props(kv: &[(&str, &str)]) -> Properties {
    let mut p = Properties::new();
    for (k, v) in kv {
        p.insert(*k, *v);
    }
    p
}

pub fn input_f32(buffer: &mut pw::buffer::Buffer, out: &mut Vec<f32>) {
    out.clear();
    let datas = buffer.datas_mut();
    let Some(d) = datas.first_mut() else { return };
    let (off, size) = (d.chunk().offset() as usize, d.chunk().size() as usize);
    if let Some(bytes) = d.data() {
        let end = (off + size).min(bytes.len());
        for b in bytes[off.min(end)..end].chunks_exact(4) {
            out.push(f32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        }
    }
}

/// Fill an output buffer of `ch` channels from `ring` (proc::fill). Returns (had enough, frames dropped).
fn output_f32(buffer: &mut pw::buffer::Buffer, ring: &mut Ring, started: &mut bool, target: usize, scratch: &mut Vec<f32>) -> (bool, usize) {
    let req = buffer.requested() as usize;
    let ch = ring.ch;
    let stride = 4 * ch;
    let datas = buffer.datas_mut();
    let Some(d) = datas.first_mut() else { return (true, 0) };
    let mut frames = 0;
    let mut r = (true, 0);
    if let Some(bytes) = d.data() {
        let cap = bytes.len() / stride;
        frames = if req == 0 { cap.min(1024) } else { req.min(cap) };
        scratch.resize(frames * ch, 0.0);
        r = fill(&mut scratch[..frames * ch], ring, started, target);
        for (i, v) in scratch[..frames * ch].iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
    }
    let c = d.chunk_mut();
    *c.offset_mut() = 0;
    *c.stride_mut() = stride as i32;
    *c.size_mut() = (frames * stride) as u32;
    r
}

/// A stream and its callbacks; the callbacks go first (dropped before the stream they hang on)
struct Live<D> {
    _l: pw::stream::StreamListener<D>,
    _s: pw::stream::Stream,
}

/// The audio devices there are now, kept by the registry watch
type Known = Rc<RefCell<std::collections::BTreeMap<u32, Node>>>;

/// The mic and the headphones in use, and what would make them change
#[derive(Default)]
struct Devs {
    mic: Option<Live<(usize, crate::proc::MicDown)>>,
    mon: Option<Live<Vec<f32>>>,
    /// node names in use
    using: [Option<String>; 2],
    /// per class: (the one chosen and missing, nothing in use)
    waiting: [(Option<String>, bool); 2],
}

const CLASSES: [&str; 2] = ["Audio/Source", "Audio/Sink"];

/// Make the four streams and run until the process ends. The virtual mic and "stream-mic 電腦聲音" are made once and
/// stay: OBS does not find "stream-mic" again once it has gone (recording went silent for good when a mic was
/// unplugged and every stream was made again, tools/func_test.py and obs_rec_test.sh on a Linux test PC). A device unplugged,
/// plugged in or chosen in the page makes only the mic's and the headphones' streams again.
pub fn run(shared: Arc<Shared>, opts: Opts) -> Result<(), String> {
    pw::init();
    let mainloop = pw::main_loop::MainLoop::new(None).map_err(|e| e.to_string())?;
    let context = pw::context::Context::new(&mainloop).map_err(|e| e.to_string())?;
    let core = context.connect(None).map_err(|e| e.to_string())?;
    // PipeWire gone (restarted or stopped): end this run, and the audio thread starts another (lib.rs), which makes
    // all four streams on the new PipeWire. Without this the main loop went on over a dead connection: no mic, no
    // headphones, no virtual mic, until stream-mic itself was restarted (2026-11-09, a sample-rate rule restarting
    // PipeWire). The lost connection is the core's EPIPE; other errors come on the core too and are not fatal (a
    // device unplugged gives "unknown resource", -ENOENT: starting again then would make the virtual mic anew, which
    // OBS does not find again)
    let weak = mainloop.downgrade();
    let _core_l = core
        .add_listener_local()
        .error(move |id, _seq, res, msg| {
            if id == pw::core::PW_ID_CORE && res == -32 {
                eprintln!("PipeWire: {msg} ({res}): connecting again");
                if let Some(ml) = weak.upgrade() {
                    ml.quit();
                }
            }
        })
        .register();

    let all = nodes(&core, &mainloop).map_err(|e| e.to_string())?;
    let (opt_mic, opt_out, quantum) = (opts.mic.clone(), opts.out.clone(), opts.quantum);
    if all.iter().any(|n| n.name == SOURCE_NAME) {
        return Err("a node named stream-mic is already there: is another stream-mic running?".into());
    }

    let rt = Rc::new(RefCell::new(Rt::new(shared.clone(), opts.quantum as usize, true)?));
    let latency = format!("{}/{}", opts.quantum, RATE);
    let flags = pw::stream::StreamFlags::MAP_BUFFERS | pw::stream::StreamFlags::RT_PROCESS;

    // the mic: its own channels, made mono here (proc::MicDown), not by PipeWire, whose downmix averages them all (an
    // interface's loopback mixes too: the io44's eight channels gave the mic at 1/8 with the computer's sound in it)
    let mic_stream = {
        let (core, rt, latency) = (core.clone(), rt.clone(), latency.clone());
        move |mic: &Node| -> Result<Live<(usize, crate::proc::MicDown)>, String> {
    let s_mic = pw::stream::Stream::new(
        &core,
        "stream-mic input",
        props(&[
            ("media.type", "Audio"),
            ("media.category", "Capture"),
            ("media.role", "Communication"),
            ("node.name", "stream-mic-input"),
            ("node.description", "stream-mic 收音"),
            ("node.latency", &latency),
            // all four in one driver group: no drift between them (pw-loopback does the same)
            ("node.group", "stream-mic"),
            ("target.object", &mic.name),
            ("node.dont-reconnect", "true"),
            // the mic gone: stay unlinked, never fall back to the default (which may be our own "stream-mic")
            ("node.dont-fallback", "true"),
            // a sink as the mic: its monitor (testing, or a program's output as the voice)
            ("stream.capture.sink", if mic.class.starts_with("Audio/Sink") { "true" } else { "false" }),
            ("stream.dont-remix", "true"),
        ]),
    )
    .map_err(|e| e.to_string())?;
    let r = rt.clone();
    // (channels as negotiated, the downmix)
    let _l_mic = s_mic
        .add_local_listener_with_user_data((1usize, crate::proc::MicDown::default()))
        .param_changed(|_, (ch, _), id, param| {
            let Some(param) = param else { return };
            if id != spa::param::ParamType::Format.as_raw() {
                return;
            }
            let mut info = spa::param::audio::AudioInfoRaw::new();
            if info.parse(param).is_ok() {
                *ch = (info.channels() as usize).max(1);
                eprintln!("mic: {} channels", *ch);
            }
        })
        .process(move |s, (ch, down)| {
            if let Some(mut b) = s.dequeue_buffer() {
                let mut rt = r.borrow_mut();
                let mut x = std::mem::take(&mut rt.mic);
                input_f32(&mut b, &mut x);
                down.apply(&mut x, *ch);
                // the graph may hand more than the chain was prepared for
                for c in x.chunks(MAX_BLOCK) {
                    rt.mic_block(c);
                }
                rt.mic = x;
            }
        })
        .register()
        .map_err(|e| e.to_string())?;
    let p = format(0);
    s_mic
        .connect(spa::utils::Direction::Input, None, flags | pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::DONT_RECONNECT, &mut [Pod::from_bytes(&p).unwrap()])
        .map_err(|e| e.to_string())?;
    Ok(Live { _l: _l_mic, _s: s_mic })
        }
    };

    // "stream-mic 電腦聲音": a virtual sink, stereo
    let s_pc = pw::stream::Stream::new(
        &core,
        "stream-mic pc audio",
        props(&[
            ("media.class", "Audio/Sink"),
            ("node.name", SINK_NAME),
            ("node.description", crate::proc::sink_desc()),
            ("audio.position", "FL,FR"),
            ("node.latency", &latency),
            // all four in one driver group: no drift between them (pw-loopback does the same)
            ("node.group", "stream-mic"),
        ]),
    )
    .map_err(|e| e.to_string())?;
    let r = rt.clone();
    let _l_pc = s_pc
        .add_local_listener_with_user_data(Vec::<f32>::with_capacity(MAX_BLOCK * 2))
        .process(move |s, buf| {
            if let Some(mut b) = s.dequeue_buffer() {
                input_f32(&mut b, buf);
                let mut rt = r.borrow_mut();
                rt.pc.push(buf);
                // the mic takes these as it goes; without a mic running they would pile up
                let keep = rt.target * 4;
                rt.pc.trim(keep);
                // no mic at all: the computer's sound still goes on to the virtual mic and the headphones (on Linux
                // they hear it only through us), with silence for the voice
                if !rt.shared.mic_live.load(Ordering::Acquire) {
                    let n = buf.len() / 2;
                    let mut z = std::mem::take(&mut rt.mic);
                    z.clear();
                    z.resize(n, 0.0);
                    for c in z.chunks(MAX_BLOCK) {
                        rt.mic_block(c);
                    }
                    rt.mic = z;
                }
            }
        })
        .register()
        .map_err(|e| e.to_string())?;
    let p = format(2);
    s_pc.connect(spa::utils::Direction::Input, None, flags | pw::stream::StreamFlags::AUTOCONNECT, &mut [Pod::from_bytes(&p).unwrap()])
        .map_err(|e| e.to_string())?;

    // "stream-mic": the virtual source, stereo
    let s_src = pw::stream::Stream::new(
        &core,
        "stream-mic",
        props(&[
            ("media.class", "Audio/Source"),
            ("node.name", SOURCE_NAME),
            ("node.description", "stream-mic"),
            ("audio.position", "FL,FR"),
            ("node.latency", &latency),
            // all four in one driver group: no drift between them (pw-loopback does the same)
            ("node.group", "stream-mic"),
        ]),
    )
    .map_err(|e| e.to_string())?;
    let r = rt.clone();
    let _l_src = s_src
        .add_local_listener_with_user_data(Vec::<f32>::with_capacity(MAX_BLOCK * 2))
        .process(move |s, scratch| {
            if let Some(mut b) = s.dequeue_buffer() {
                let mut rt = r.borrow_mut();
                let Rt { send, send_on, target, shared, .. } = &mut *rt;
                let (ok, dropped) = output_f32(&mut b, send, send_on, *target, scratch);
                if !ok {
                    shared.count.send_under.fetch_add(1, Ordering::Relaxed);
                }
                shared.count.send_drop.fetch_add(dropped as u64, Ordering::Relaxed);
            }
        })
        .register()
        .map_err(|e| e.to_string())?;
    let p = format(2);
    s_src
        .connect(spa::utils::Direction::Output, None, flags | pw::stream::StreamFlags::AUTOCONNECT, &mut [Pod::from_bytes(&p).unwrap()])
        .map_err(|e| e.to_string())?;

    // the headphones: a named hardware sink, never the default (which may be our own sink)
    let mon_stream = {
        let (core, rt, latency) = (core.clone(), rt.clone(), latency.clone());
        move |out: &Node| -> Result<Live<Vec<f32>>, String> {
    let s_mon = pw::stream::Stream::new(
        &core,
        "stream-mic monitor",
        props(&[
            ("media.type", "Audio"),
            ("media.category", "Playback"),
            ("media.role", "Music"),
            ("node.name", "stream-mic-monitor"),
            ("node.description", "stream-mic 監聽"),
            ("node.latency", &latency),
            // all four in one driver group: no drift between them (pw-loopback does the same)
            ("node.group", "stream-mic"),
            ("target.object", &out.name),
            ("node.dont-reconnect", "true"),
            // the headphones gone: stay unlinked, never fall back to the default sink, which is often our own
            // "stream-mic 電腦聲音" (feedback; seen on a Linux test PC when the target vanished)
            ("node.dont-fallback", "true"),
        ]),
    )
    .map_err(|e| e.to_string())?;
    let r = rt.clone();
    let _l_mon = s_mon
        .add_local_listener_with_user_data(Vec::<f32>::with_capacity(MAX_BLOCK * 2))
        .process(move |s, scratch| {
            if let Some(mut b) = s.dequeue_buffer() {
                let mut rt = r.borrow_mut();
                let Rt { mon, mon_on, target, shared, .. } = &mut *rt;
                let (ok, dropped) = output_f32(&mut b, mon, mon_on, *target, scratch);
                if !ok {
                    shared.count.mon_under.fetch_add(1, Ordering::Relaxed);
                }
                shared.count.mon_drop.fetch_add(dropped as u64, Ordering::Relaxed);
            }
        })
        .register()
        .map_err(|e| e.to_string())?;
    let p = format(2);
    s_mon
        .connect(spa::utils::Direction::Output, None, flags | pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::DONT_RECONNECT, &mut [Pod::from_bytes(&p).unwrap()])
        .map_err(|e| e.to_string())?;
    Ok(Live { _l: _l_mon, _s: s_mon })
        }
    };

    // pick the mic and the headphones from what there is, (re)make their streams, tell the page
    let known: Known = Rc::default();
    for (i, n) in all.iter().enumerate() {
        known.borrow_mut().insert(u32::MAX - i as u32, n.clone());
    }
    let devs: Rc<RefCell<Devs>> = Rc::default();
    let connect = {
        let (known, devs, shared) = (known.clone(), devs.clone(), shared.clone());
        move || -> Result<(), String> {
            let all: Vec<Node> = known.borrow().values().cloned().collect();
            let (want_mic, want_out) = {
                let c = shared.cfg.lock().unwrap();
                (c.mic.clone().or(opt_mic.clone()), c.out.clone().or(opt_out.clone()))
            };
            // a device chosen before and not there now (unplugged): the system's choice, and the page says so
            let mic_missing = want_mic.clone().filter(|w| pick(&all, CLASSES[0], Some(w)).is_none());
            let out_missing = want_out.clone().filter(|w| pick(&all, CLASSES[1], Some(w)).is_none());
            // none at all (the only mic unplugged): the virtual mic stays, silent, until one comes
            let mic = pick(&all, CLASSES[0], want_mic.as_deref()).or_else(|| pick(&all, CLASSES[0], None));
            let out = pick(&all, CLASSES[1], want_out.as_deref()).or_else(|| pick(&all, CLASSES[1], None));
            let desc = |n: &Option<Node>| n.as_ref().map_or("none".to_string(), |n| format!("{} ({})", n.desc, n.name));
            eprintln!("mic: {}\nheadphones: {}", desc(&mic), desc(&out));
            let mut d = devs.borrow_mut();
            let name = |n: &Option<Node>| n.as_ref().map(|n| n.name.clone());
            if d.using[0] != name(&mic) || d.mic.is_none() != mic.is_none() {
                d.mic = None;
                d.mic = mic.as_ref().map(&mic_stream).transpose()?;
            }
            if d.using[1] != name(&out) || d.mon.is_none() != out.is_none() {
                d.mon = None;
                d.mon = out.as_ref().map(&mon_stream).transpose()?;
            }
            d.using = [name(&mic), name(&out)];
            // never rt here: the data thread holds it (RefCell already borrowed, the engine aborted)
            shared.mic_live.store(d.mic.is_some(), Ordering::Release);
            d.waiting = [(mic_missing.clone(), mic.is_none()), (out_missing.clone(), out.is_none())];
            let dev = |n: &Option<Node>| n.as_ref().map(|n| serde_json::json!({ "name": n.name, "desc": n.desc }));
            let list = |c: &str| all.iter().filter(|n| n.class.starts_with(c) && !ours(n)).map(|n| serde_json::json!({ "name": n.name, "desc": n.desc })).collect::<Vec<_>>();
            *shared.devices.lock().unwrap() = serde_json::json!({
                "mic": dev(&mic),
                "out": dev(&out),
                "mics": list(CLASSES[0]),
                "mic_missing": mic_missing,
                "out_missing": out_missing,
                "outs": list(CLASSES[1]),
            });
            Ok(())
        }
    };
    connect()?;

    // the last guard against feedback: a link from our headphone stream into our own sink, or from our virtual mic
    // into our own input, is removed as soon as it appears, whoever made it
    //
    // and the devices: the mic or the headphones in use unplugged, or the chosen one (or any, with none) plugged
    // in: pick again (on the timer below, outside the registry's callback)
    let registry = core.get_registry().map_err(|e| e.to_string())?;
    let registry = Rc::new(registry);
    let names: Rc<RefCell<std::collections::HashMap<u32, String>>> = Rc::default();
    let (reg, nm) = (Rc::downgrade(&registry), names.clone());
    let (kn, dv, sh) = (known.clone(), devs.clone(), shared.clone());
    let (kn2, nm2, dv2, sh2) = (known.clone(), names.clone(), devs.clone(), shared.clone());
    let _guard = registry
        .add_listener_local()
        .global(move |g| {
            let Some(p) = g.props else { return };
            if let Some(n) = p.get("node.name") {
                nm.borrow_mut().insert(g.id, n.to_string());
                let class = p.get("media.class").unwrap_or("");
                if !(class.starts_with(CLASSES[0]) || class.starts_with(CLASSES[1])) {
                    return;
                }
                let node = Node {
                    name: n.to_string(),
                    desc: p.get("node.description").or(p.get("node.nick")).unwrap_or("").to_string(),
                    class: class.to_string(),
                    prio: p.get("priority.session").and_then(|v| v.parse().ok()).unwrap_or(0),
                };
                // the first listing (from nodes()) was keyed apart: the same node by its real id now
                kn.borrow_mut().retain(|k, v| !(*k > u32::MAX - 4096 && v.name == node.name));
                kn.borrow_mut().insert(g.id, node.clone());
                if !ours(&node) {
                    let d = dv.borrow();
                    for (c, (missing, none)) in CLASSES.iter().zip(&d.waiting) {
                        // the chosen one by name, whatever its class (a sink's monitor can be the mic)
                        if missing.as_deref() == Some(n) || (*none && class.starts_with(c)) {
                            eprintln!("{n} is here: picking the devices again");
                            sh.restart.store(true, Ordering::Release);
                        }
                    }
                }
                return;
            }
            let (Some(o), Some(i)) = (p.get("link.output.node"), p.get("link.input.node")) else { return };
            let name = |s: &str| s.parse::<u32>().ok().and_then(|id| nm.borrow().get(&id).cloned()).unwrap_or_default();
            let (o, i) = (name(o), name(i));
            if (o == "stream-mic-monitor" && i == SINK_NAME) || (o == SOURCE_NAME && i == "stream-mic-input") {
                eprintln!("removed a feedback link {o} -> {i}");
                if let Some(r) = reg.upgrade() {
                    let _ = r.destroy_global(g.id);
                }
            }
        })
        .global_remove(move |id| {
            kn2.borrow_mut().remove(&id);
            if let Some(n) = nm2.borrow_mut().remove(&id) {
                if dv2.borrow().using.iter().any(|u| u.as_deref() == Some(n.as_str())) {
                    eprintln!("{n} is gone: picking the devices again");
                    sh2.restart.store(true, Ordering::Release);
                }
            }
        })
        .register();

    // a device came or went, or the page chose another: the mic's and the headphones' streams again
    let sh = shared.clone();
    let repick = mainloop.loop_().add_timer(move |_| {
        if sh.restart.swap(false, Ordering::AcqRel) {
            if let Err(e) = connect() {
                eprintln!("stream-mic: devices: {e}");
            }
        }
    });
    repick.update_timer(Some(Duration::from_millis(200)), Some(Duration::from_millis(200))).into_result().map_err(|e| format!("{e:?}"))?;

    eprintln!("running: virtual mic \"stream-mic\", output \"stream-mic 電腦聲音\", quantum {}", quantum);
    mainloop.run();
    Ok(())
}
