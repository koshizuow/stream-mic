//! Windows (build-plan.md step 9): WASAPI in shared mode, one audio thread (MMCSS "Pro Audio").
//!
//!   mic (a capture endpoint) ──► chain ──┬──► VB-CABLE's "CABLE Input": OBS and Discord take "CABLE Output"
//!                                        └──► the headphones (a render endpoint)
//!   every program's sound but ours (process loopback, our process tree left out) ──► the virtual mic (the loopback),
//!   the reverb's music follower and loop detection
//!
//! Windows has no virtual devices without a driver, so the virtual mic is VB-CABLE (DECISIONS.md). The computer's
//! sound needs no routing: Windows plays it to the headphones itself (so it is not added there again), and process
//! loopback hears all of it but stream-mic's own output.
//!
//! The thread waits on the mic: each packet goes through the chain (proc.rs) and the two outputs are topped up to a
//! lead of a period or two. Every endpoint has its own clock (VB-CABLE's ran 0.15 % off the interface's on the Windows test PC):
//! the outputs and the computer's sound coming in each go through a resampler that follows the other clock by how
//! full its ring is (asrc.rs), about two packets (20 ms) held in each. Every device is opened at its own rate
//! (Revelator IO 44: 44.1 kHz) and asrc.rs converts, from and to the chain's 48 kHz: Windows' conversion was poor.
//!
//! Devices: endpoint ids are what the settings keep, friendly names what the page shows. A stream that fails (the
//! device unplugged) is dropped and the devices picked again; the list is looked at every second, so a chosen device
//! coming back is taken again. The page's choice sets `restart`.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use wasapi::{
    get_default_device, initialize_mta, AudioCaptureClient, AudioClient, AudioRenderClient, DeviceCollection, Direction,
    Handle, SampleType, StreamMode, WaveFormat,
};

use crate::asrc::Asrc;
use crate::proc::{ours, pick, Node, Opts, Rt, MAX_BLOCK, RATE};
use crate::state::Shared;

const CLASSES: [&str; 2] = ["Audio/Source", "Audio/Sink"];
/// VB-CABLE's render endpoint, by its friendly name ("CABLE Input (VB-Audio Virtual Cable)")
const CABLE: &str = "CABLE Input";

fn werr(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// The active endpoints: capture ones as Audio/Source, render ones as Audio/Sink; the default ones first (prio 1)
pub fn list() -> Result<Vec<Node>, String> {
    let _ = initialize_mta();
    let mut out = Vec::new();
    for (dir, class) in [(Direction::Capture, CLASSES[0]), (Direction::Render, CLASSES[1])] {
        let def = get_default_device(&dir).ok().and_then(|d| d.get_id().ok());
        let c = DeviceCollection::new(&dir).map_err(werr)?;
        for i in 0..c.get_nbr_devices().map_err(werr)? {
            let d = c.get_device_at_index(i).map_err(werr)?;
            let id = d.get_id().map_err(werr)?;
            out.push(Node {
                prio: (def.as_deref() == Some(id.as_str())) as i64,
                desc: d.get_friendlyname().unwrap_or_default(),
                name: id,
                class: class.to_string(),
            });
        }
    }
    Ok(out)
}

fn open_client(id: &str, dir: Direction) -> Result<AudioClient, String> {
    let c = DeviceCollection::new(&dir).map_err(werr)?;
    for i in 0..c.get_nbr_devices().map_err(werr)? {
        let d = c.get_device_at_index(i).map_err(werr)?;
        if d.get_id().map_err(werr)? == id {
            return d.get_iaudioclient().map_err(werr);
        }
    }
    Err(format!("{id}: not there"))
}

/// float at `rate`, `ch` channels. Windows converts the sample format and the channels (AUTOCONVERTPCM); the rate is
/// always the device's own (mix_rate): its rate conversion left images 58 dB under a 1 kHz tone, so asrc.rs does that
fn fmt(rate: usize, ch: usize) -> WaveFormat {
    WaveFormat::new(32, 32, &SampleType::Float, rate, ch, None)
}

/// The rate the device runs at in shared mode
fn mix_rate(client: &AudioClient) -> usize {
    client.get_mixformat().map(|f| f.get_samplespersec() as usize).unwrap_or(RATE as usize)
}

struct Capture {
    client: AudioClient,
    cap: AudioCaptureClient,
    event: Handle,
    ch: usize,
    rate: usize,
}

impl Capture {
    /// the mic, stereo where the endpoint has two channels (mixed down here: one mic on input 1 of a two-input
    /// interface would lose 6 dB in Windows' own downmix, the other input being silent), else mono
    fn mic(id: &str) -> Result<Capture, String> {
        let mut last = String::new();
        for ch in [2, 1] {
            let mut client = open_client(id, Direction::Capture)?;
            let rate = mix_rate(&client);
            match client.initialize_client(&fmt(rate, ch), &Direction::Capture, &StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 }) {
                Ok(()) => return Capture::start(client, ch, rate),
                Err(e) => last = werr(e),
            }
        }
        Err(last)
    }

    /// every program's sound but this process's (and its children's), at the default output's rate (what the
    /// programs are mixed at, so Windows need not convert it)
    fn loopback() -> Result<Capture, String> {
        let rate = get_default_device(&Direction::Render).ok().and_then(|d| d.get_iaudioclient().ok()).map(|c| mix_rate(&c)).unwrap_or(RATE as usize);
        let mut client = AudioClient::new_application_loopback_client(std::process::id(), false).map_err(werr)?;
        client
            .initialize_client(&fmt(rate, 2), &Direction::Capture, &StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 })
            .map_err(werr)?;
        Capture::start(client, 2, rate)
    }

    /// what is played to a render endpoint (tests: STREAM_MIC_TEST_PC, and --capture on an output)
    fn endpoint_loopback(id: &str) -> Result<Capture, String> {
        let mut client = open_client(id, Direction::Render)?;
        let rate = mix_rate(&client);
        client
            .initialize_client(&fmt(rate, 2), &Direction::Capture, &StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 })
            .map_err(werr)?;
        Capture::start(client, 2, rate)
    }

    fn start(client: AudioClient, ch: usize, rate: usize) -> Result<Capture, String> {
        let event = client.set_get_eventhandle().map_err(werr)?;
        let cap = client.get_audiocaptureclient().map_err(werr)?;
        client.start_stream().map_err(werr)?;
        Ok(Capture { client, cap, event, ch, rate })
    }

    /// everything waiting, interleaved f32, appended to `out`; Err when the device is gone
    fn read(&self, bytes: &mut Vec<u8>, out: &mut Vec<f32>) -> Result<(), String> {
        while let Some(n) = self.cap.get_next_packet_size().map_err(werr)? {
            if n == 0 {
                break;
            }
            bytes.resize(n as usize * 4 * self.ch, 0);
            let (got, info) = self.cap.read_from_device(bytes).map_err(werr)?;
            let k = got as usize * self.ch;
            if info.flags.silent {
                out.extend(std::iter::repeat(0.0).take(k));
            } else {
                out.extend(bytes[..k * 4].chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])));
            }
        }
        Ok(())
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.client.stop_stream();
    }
}

struct Render {
    client: AudioClient,
    ren: AudioRenderClient,
    /// frames kept waiting in the device's buffer: three periods. Written only when the mic's packet comes, so the
    /// two cycles' phases drift: with a period and a half, the buffer ran dry now and then on the Windows test PC (Windows played
    /// silence, took less of ours, the ring grew and the resampler ran to its limit)
    lead: usize,
    /// this device's clock against the mic's
    asrc: Asrc,
    /// STREAM_MIC_DEBUG: (calls, least padding, most, frames written, name)
    debug: Option<(usize, usize, usize, usize, String)>,
}

impl Render {
    fn open(id: &str) -> Result<Render, String> {
        let mut client = open_client(id, Direction::Render)?;
        let rate = mix_rate(&client);
        // 40 ms of buffer, polled: written on the mic's cycle
        client
            .initialize_client(&fmt(rate, 2), &Direction::Render, &StreamMode::PollingShared { autoconvert: true, buffer_duration_hns: 400_000 })
            .map_err(werr)?;
        let ren = client.get_audiorenderclient().map_err(werr)?;
        let (def, _min) = client.get_device_period().map_err(werr)?;
        let period = (def as usize * rate / 10_000_000).max(128);
        client.start_stream().map_err(werr)?;
        // from the chain's 48 kHz to the device's rate, following its clock
        let debug = std::env::var_os("STREAM_MIC_DEBUG").map(|_| (0, usize::MAX, 0, 0, format!("{id} {rate} Hz period {period}")));
        Ok(Render { client, ren, lead: period * 3, asrc: Asrc::new(2, rate as f64, RATE as f64 / rate as f64), debug })
    }

    /// top the device's buffer up to `lead` from `ring`, through the resampler; (had enough, dropped), Err when it is
    /// gone
    fn top_up(&mut self, ring: &mut crate::state::Ring, target: usize, scratch: &mut Vec<f32>, bytes: &mut Vec<u8>) -> Result<(bool, usize), String> {
        let pad = self.client.get_current_padding().map_err(werr)? as usize;
        let space = self.client.get_available_space_in_frames().map_err(werr)? as usize;
        let n = self.lead.saturating_sub(pad).min(space);
        if let Some(d) = self.debug.as_mut() {
            d.0 += 1;
            d.1 = d.1.min(pad);
            d.2 = d.2.max(pad);
            d.3 += n;
            if d.0 % 100 == 0 {
                eprintln!("{}: pad {}..{} lead {} wrote {} in 100 calls, ring {} drift {:.0} ppm", d.4, d.1, d.2, self.lead, d.3, ring.frames(), (self.asrc.drift() - 1.0) * 1e6);
                (d.1, d.2, d.3) = (usize::MAX, 0, 0);
            }
        }
        if n == 0 {
            return Ok((true, 0));
        }
        scratch.resize(n * 2, 0.0);
        let r = self.asrc.pull(ring, &mut scratch[..n * 2], target);
        bytes.clear();
        for v in &scratch[..n * 2] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        self.ren.write_to_device(n, bytes, None).map_err(werr)?;
        Ok(r)
    }
}

impl Drop for Render {
    fn drop(&mut self) {
        let _ = self.client.stop_stream();
    }
}

/// What is open now, and what the page is told
#[derive(Default)]
struct Devs {
    mic: Option<Capture>,
    /// the mic at another rate than 48 kHz: converted (no clock to follow: the mic is the one the rest follow)
    mic_conv: Option<(crate::state::Ring, Asrc)>,
    /// the computer sound's rate when it was opened (its resampler is made for it)
    lb_rate: usize,
    mon: Option<Render>,
    cable: Option<Render>,
    lb: Option<Capture>,
    /// endpoint ids in use: mic, headphones, cable
    using: [Option<String>; 3],
}

/// Tests without a person (tools/func_test.py on the Windows test PC, which has one VB-CABLE and no way to feed a mic):
/// STREAM_MIC_TEST_CABLE names a render endpoint (part of its name) to be the virtual mic instead of VB-CABLE,
/// STREAM_MIC_TEST_PC one whose loopback is the computer's sound instead of the process loopback
fn test_dev(var: &str, all: &[Node]) -> Option<Node> {
    let w = std::env::var(var).ok()?;
    all.iter().find(|n| n.class == CLASSES[1] && n.desc.contains(&w)).cloned()
}

/// A device named by its id, or (the command line on Windows, where ids are long) by part of its name
fn pick_named(all: &[Node], class: &str, want: Option<&str>) -> Option<Node> {
    pick(all, class, want).or_else(|| want.and_then(|w| all.iter().find(|n| n.class == class && n.desc.contains(w) && !ours(n)).cloned()))
}

/// Pick the mic, the headphones and VB-CABLE from what there is now, open what changed, tell the page
fn connect(shared: &Shared, devs: &mut Devs, all: &[Node], opts: &Opts) {
    let (want_mic, want_out) = {
        let c = shared.cfg.lock().unwrap();
        (c.mic.clone().or(opts.mic.clone()), c.out.clone().or(opts.out.clone()))
    };
    let mic_missing = want_mic.clone().filter(|w| pick_named(all, CLASSES[0], Some(w)).is_none());
    let out_missing = want_out.clone().filter(|w| pick_named(all, CLASSES[1], Some(w)).is_none());
    let mic = pick_named(all, CLASSES[0], want_mic.as_deref()).or_else(|| pick(all, CLASSES[0], None));
    let out = pick_named(all, CLASSES[1], want_out.as_deref()).or_else(|| pick(all, CLASSES[1], None));
    // "CABLE Input"; else, should the name be translated, VB-CABLE's render endpoint but its 16-channel one
    let vb = |n: &&Node| n.class == CLASSES[1] && n.desc.contains("VB-Audio Virtual Cable");
    let cable = test_dev("STREAM_MIC_TEST_CABLE", all)
        .or_else(|| all.iter().filter(vb).find(|n| n.desc.starts_with(CABLE)).or_else(|| all.iter().filter(vb).find(|n| !n.desc.contains("16"))).cloned());
    let name = |n: &Option<Node>| n.as_ref().map(|n| n.name.clone());
    let now = [name(&mic), name(&out), name(&cable)];
    let desc = |n: &Option<Node>| n.as_ref().map_or("none".to_string(), |n| n.desc.clone());
    if now != devs.using || devs.mic.is_none() != mic.is_none() || devs.mon.is_none() != out.is_none() || devs.cable.is_none() != cable.is_none() {
        eprintln!("mic: {}\nheadphones: {}\nvirtual mic: {}", desc(&mic), desc(&out), cable.as_ref().map_or("VB-CABLE not installed".into(), |c| c.desc.clone()));
    }
    let report = |what: &str, e: String| eprintln!("stream-mic: {what}: {e}");
    if now[0] != devs.using[0] || devs.mic.is_none() {
        devs.mic = None;
        devs.mic = mic.as_ref().and_then(|m| Capture::mic(&m.name).map_err(|e| report("mic", e)).ok());
        devs.mic_conv = devs.mic.as_ref().filter(|m| m.rate != RATE as usize).map(|m| {
            (crate::state::Ring::new(m.rate / 2, 1), Asrc::new(1, RATE as f64, m.rate as f64 / RATE as f64))
        });
    }
    if now[1] != devs.using[1] || devs.mon.is_none() {
        devs.mon = None;
        devs.mon = out.as_ref().and_then(|o| Render::open(&o.name).map_err(|e| report("headphones", e)).ok());
    }
    if now[2] != devs.using[2] || devs.cable.is_none() {
        devs.cable = None;
        devs.cable = cable.as_ref().and_then(|c| Render::open(&c.name).map_err(|e| report("VB-CABLE", e)).ok());
    }
    if devs.lb.is_none() {
        devs.lb = match test_dev("STREAM_MIC_TEST_PC", all) {
            Some(n) => Capture::endpoint_loopback(&n.name),
            None => Capture::loopback(),
        }
        .map_err(|e| report("computer sound", e))
        .ok();
        devs.lb_rate = devs.lb.as_ref().map_or(0, |l| l.rate);
    }
    devs.using = now;
    shared.mic_live.store(devs.mic.is_some(), Ordering::Release);
    let dev = |n: &Option<Node>| n.as_ref().map(|n| serde_json::json!({ "name": n.name, "desc": n.desc }));
    let list = |c: &str| all.iter().filter(|n| n.class == c && !ours(n)).map(|n| serde_json::json!({ "name": n.name, "desc": n.desc })).collect::<Vec<_>>();
    *shared.devices.lock().unwrap() = serde_json::json!({
        "mic": dev(&mic),
        "out": dev(&out),
        "mics": list(CLASSES[0]),
        "outs": list(CLASSES[1]),
        "mic_missing": mic_missing,
        "out_missing": out_missing,
        // Windows: the virtual mic is VB-CABLE; without it there is none (the page says how to get it)
        "cable": dev(&cable),
        "loopback_ok": devs.lb.is_some(),
    });
}

/// The audio thread: never returns but on a failure to start
pub fn run(shared: Arc<Shared>, opts: Opts) -> Result<(), String> {
    initialize_mta().ok().map_err(werr)?;
    mmcss();
    // two WASAPI periods (10 ms) held in each ring: one is how far a ring's level jumps as packets come (asrc.rs)
    let target = (opts.quantum as usize).max(960);
    let mut rt = Rt::new(shared.clone(), target, false)?;
    let mut devs = Devs::default();
    let mut all = list()?;
    connect(&shared, &mut devs, &all, &opts);
    // the computer's sound: from its rate to 48 kHz, following its clock, two of its packets held
    let mut pc_rate = 0;
    eprintln!("running: virtual mic VB-CABLE, computer sound by process loopback");

    let (mut bytes, mut out_bytes) = (Vec::<u8>::new(), Vec::<u8>::new());
    let (mut mic, mut pc, mut scratch) = (Vec::<f32>::with_capacity(MAX_BLOCK * 2), Vec::<f32>::with_capacity(MAX_BLOCK * 2), Vec::<f32>::new());
    let mut looked = Instant::now();
    // without a mic: the chain runs on silence at the clock's pace, so the computer's sound still goes out
    let mut idle_from = Instant::now();
    let mut idle_done = 0u64;
    // a two-channel mic made mono (proc::MicDown: an unused input left out)
    let mut mic_down = crate::proc::MicDown::default();
    loop {
        let mut lost = false;
        match &devs.mic {
            Some(m) => {
                let _ = m.event.wait_for_event(50);
            }
            None => std::thread::sleep(Duration::from_millis(5)),
        }
        if devs.lb_rate != 0 && devs.lb_rate != pc_rate {
            pc_rate = devs.lb_rate;
            rt.pc_asrc = Some(Asrc::new(2, RATE as f64, pc_rate as f64 / RATE as f64));
            rt.pc_lead = pc_rate / 50;
        }
        // the computer's sound first: the mic's block takes from it
        pc.clear();
        if let Some(l) = &devs.lb {
            if let Err(e) = l.read(&mut bytes, &mut pc) {
                eprintln!("computer sound: {e}");
                devs.lb = None;
            }
        }
        if !pc.is_empty() {
            rt.pc.push(&pc);
            let keep = rt.pc_lead * 4;
            rt.pc.trim(keep);
        }
        mic.clear();
        if let Some(m) = &devs.mic {
            match m.read(&mut bytes, &mut mic) {
                Ok(()) => {
                    mic_down.apply(&mut mic, m.ch as usize);
                    if let Some((ring, conv)) = devs.mic_conv.as_mut() {
                        ring.push(&mic);
                        mic.clear();
                        conv.convert(ring, &mut mic);
                    }
                }
                Err(e) => {
                    eprintln!("mic: {e}");
                    devs.mic = None;
                    lost = true;
                }
            }
            idle_from = Instant::now();
            idle_done = 0;
        } else {
            let due = (idle_from.elapsed().as_secs_f64() * RATE as f64) as u64;
            mic.resize((due - idle_done).min(MAX_BLOCK as u64) as usize, 0.0);
            idle_done += mic.len() as u64;
        }
        for c in mic.chunks(MAX_BLOCK) {
            rt.mic_block(c);
        }
        // the two outputs
        let Rt { send, mon, target: t, shared: sh, .. } = &mut rt;
        if let Some(c) = devs.cable.as_mut() {
            match c.top_up(send, *t, &mut scratch, &mut out_bytes) {
                Ok((ok, dropped)) => {
                    sh.count.ratio_ppm[0].store((c.asrc.drift() * 1e6).round() as u64, Ordering::Relaxed);
                    if !ok {
                        sh.count.send_under.fetch_add(1, Ordering::Relaxed);
                    }
                    sh.count.send_drop.fetch_add(dropped as u64, Ordering::Relaxed);
                }
                Err(e) => {
                    eprintln!("VB-CABLE: {e}");
                    devs.cable = None;
                    lost = true;
                }
            }
        } else {
            // nobody takes it: keep the ring from piling up
            send.trim(*t);
        }
        if let Some(o) = devs.mon.as_mut() {
            match o.top_up(mon, *t, &mut scratch, &mut out_bytes) {
                Ok((ok, dropped)) => {
                    sh.count.ratio_ppm[1].store((o.asrc.drift() * 1e6).round() as u64, Ordering::Relaxed);
                    if !ok {
                        sh.count.mon_under.fetch_add(1, Ordering::Relaxed);
                    }
                    sh.count.mon_drop.fetch_add(dropped as u64, Ordering::Relaxed);
                }
                Err(e) => {
                    eprintln!("headphones: {e}");
                    devs.mon = None;
                    lost = true;
                }
            }
        } else {
            mon.trim(*t);
        }
        // a device gone, the page's choice, or a second gone by: look at the devices again
        let asked = shared.restart.swap(false, Ordering::AcqRel);
        if lost || asked || looked.elapsed() > Duration::from_secs(1) {
            looked = Instant::now();
            if let Ok(now) = list() {
                let changed = now.len() != all.len() || now.iter().zip(&all).any(|(a, b)| a.name != b.name || a.prio != b.prio);
                all = now;
                if lost || asked || changed || devs.lb.is_none() {
                    connect(&shared, &mut devs, &all, &opts);
                }
            }
        }
    }
}

/// Each endpoint's own format (the shared-mode mix format), for `--formats`
pub fn formats() -> Result<Vec<(Node, String)>, String> {
    let _ = initialize_mta();
    let mut out = Vec::new();
    for n in list()? {
        let dir = if n.class == CLASSES[0] { Direction::Capture } else { Direction::Render };
        let f = open_client(&n.name, dir)
            .and_then(|c| c.get_mixformat().map_err(werr))
            .map(|f| format!("{} Hz, {} ch, {} bit", f.get_samplespersec(), f.get_nchannels(), f.get_bitspersample()))
            .unwrap_or_else(|e| e);
        out.push((n, f));
    }
    Ok(out)
}

/// For tests (the way pw-record is used on Linux): `secs` of a capture endpoint, by id or by a part of its name, to a
/// float stereo WAV at the device's own rate
pub fn capture_to_wav(dev: &str, secs: f64, path: &str) -> Result<(), String> {
    initialize_mta().ok().map_err(werr)?;
    let all = list()?;
    // a capture endpoint, else an output's loopback (what is played to it)
    let n = all
        .iter()
        .find(|n| n.class == CLASSES[0] && (n.name == dev || n.desc.contains(dev)))
        .or_else(|| all.iter().find(|n| n.class == CLASSES[1] && (n.name == dev || n.desc.contains(dev))))
        .ok_or(format!("no device {dev}"))?;
    let c = if n.class == CLASSES[1] {
        Capture::endpoint_loopback(&n.name)?
    } else {
        let mut client = open_client(&n.name, Direction::Capture)?;
        let rate = mix_rate(&client);
        client
            .initialize_client(&fmt(rate, 2), &Direction::Capture, &StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 })
            .map_err(werr)?;
        Capture::start(client, 2, rate)?
    };
    let rate = c.rate;
    let want = (secs * rate as f64) as usize * 2;
    let (mut bytes, mut x) = (Vec::new(), Vec::with_capacity(want));
    while x.len() < want {
        let _ = c.event.wait_for_event(100);
        c.read(&mut bytes, &mut x)?;
    }
    x.truncate(want);
    let mut w = Vec::with_capacity(44 + x.len() * 4);
    let data = (x.len() * 4) as u32;
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&(rate as u32).to_le_bytes());
    w.extend_from_slice(&(rate as u32 * 8).to_le_bytes());
    w.extend_from_slice(&8u16.to_le_bytes());
    w.extend_from_slice(&32u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data.to_le_bytes());
    for v in &x {
        w.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, w).map_err(werr)
}

/// For tests: a WAV (16-bit or float, any rate, mono or stereo) played on a render endpoint (by id or part of its
/// name), looped for `secs`, converted to the device's rate here
pub fn play_wav(dev: &str, path: &str, secs: f64) -> Result<(), String> {
    initialize_mta().ok().map_err(werr)?;
    let b = std::fs::read(path).map_err(werr)?;
    // the chunks: fmt and data
    let (mut i, mut fmtc, mut data) = (12, None, None);
    while i + 8 <= b.len() {
        let id = &b[i..i + 4];
        let n = u32::from_le_bytes([b[i + 4], b[i + 5], b[i + 6], b[i + 7]]) as usize;
        if id == b"fmt " {
            fmtc = Some(&b[i + 8..i + 8 + n]);
        } else if id == b"data" {
            data = Some(&b[i + 8..(i + 8 + n).min(b.len())]);
        }
        i += 8 + n + (n & 1);
    }
    let (f, d) = (fmtc.ok_or("no fmt")?, data.ok_or("no data")?);
    let tag = u16::from_le_bytes([f[0], f[1]]);
    let ch = u16::from_le_bytes([f[2], f[3]]) as usize;
    let rate = u32::from_le_bytes([f[4], f[5], f[6], f[7]]) as f64;
    let bits = u16::from_le_bytes([f[14], f[15]]);
    let x: Vec<f32> = match (tag, bits) {
        (1, 16) | (0xFFFE, 16) => d.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect(),
        (3, 32) | (0xFFFE, 32) => d.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect(),
        _ => return Err(format!("format {tag} with {bits} bits")),
    };
    let stereo: Vec<f32> = if ch == 1 { x.iter().flat_map(|v| [*v, *v]).collect() } else { x.chunks_exact(ch).flat_map(|c| [c[0], c[1]]).collect() };
    let n = list()?.into_iter().find(|n| n.class == CLASSES[1] && (n.name == dev || n.desc.contains(dev))).ok_or(format!("no output {dev}"))?;
    let out = Render::open(&n.name)?;
    let dev_rate = mix_rate(&out.client) as f64;
    // the file's rate to the device's, all at once
    let mut conv = Asrc::new(2, dev_rate, rate / dev_rate);
    let mut ring = crate::state::Ring::new(stereo.len() / 2 + 64, 2);
    ring.push(&stereo);
    ring.push(&[0.0; 128]);
    let mut y = Vec::new();
    conv.convert(&mut ring, &mut y);
    let frames = y.len() / 2;
    let (mut pos, mut bytes) = (0usize, Vec::new());
    let end = std::time::Instant::now() + Duration::from_secs_f64(secs);
    while std::time::Instant::now() < end {
        let pad = out.client.get_current_padding().map_err(werr)? as usize;
        let n = out.lead.saturating_sub(pad);
        if n > 0 {
            bytes.clear();
            for _ in 0..n {
                bytes.extend_from_slice(&y[2 * pos].to_le_bytes());
                bytes.extend_from_slice(&y[2 * pos + 1].to_le_bytes());
                pos = (pos + 1) % frames;
            }
            out.ren.write_to_device(n, &bytes, None).map_err(werr)?;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}

/// Ask Windows to schedule this thread as audio (MMCSS "Pro Audio")
fn mmcss() {
    use windows::core::w;
    use windows::Win32::System::Threading::AvSetMmThreadCharacteristicsW;
    let mut index = 0u32;
    if unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut index) }.is_err() {
        eprintln!("MMCSS: not available, running at normal priority");
    }
}
