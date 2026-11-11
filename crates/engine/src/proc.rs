//! What the audio thread does the same on every platform (Linux: audio.rs, PipeWire; Windows: audio_win.rs,
//! WASAPI): its state, the work per block of the mic (the chain, the computer's sound mixed in, the meters, the taps
//! for loop detection and auto setup), the rings to the outputs, and the devices as the page sees them.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use stream_mic_chain::Chain;
use stream_mic_dsp::svf::{Coefs, Kind, Svf};

use crate::dnworker::{self, DnLink};
use crate::state::{db, Cfg, Delay, Monitor, Ring, Shared};

pub const RATE: u32 = 48000;
pub const MAX_BLOCK: usize = 8192;

pub struct Opts {
    /// the mic (Linux: node.name; Windows: endpoint id), or None: the system's choice
    pub mic: Option<String>,
    /// the headphones, likewise
    pub out: Option<String>,
    /// frames per cycle asked of the graph (Linux), or the buffer aimed at (Windows)
    pub quantum: u32,
}

/// An audio device: `name` is what the settings keep (Linux node.name, Windows endpoint id), `desc` what people read
#[derive(Clone, Debug)]
pub struct Node {
    pub name: String,
    pub desc: String,
    pub class: String,
    pub prio: i64,
}

/// What programs see as stream-mic's sink for the computer's sound (Linux), in the system's language: chosen when it
/// is made, since OBS and the like find a device again by it (changing it with the page's language would lose them)
pub fn sink_desc() -> &'static str {
    if system_lang() == "zh" { "stream-mic 電腦聲音" } else { "stream-mic Computer Audio" }
}

/// The system's language as the page knows them: "zh" for any Chinese, else "en"
pub fn system_lang() -> &'static str {
    #[cfg(windows)]
    let zh = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() } & 0x3ff == 0x04;
    #[cfg(not(windows))]
    let zh = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty())).unwrap_or_default().starts_with("zh");
    if zh { "zh" } else { "en" }
}

/// The language chosen in the settings, or the system's (the tray's menu; the page itself falls back to the browser's)
pub fn ui_lang(shared: &crate::state::Shared) -> &'static str {
    match shared.cfg.lock().unwrap().ui.lang.as_str() {
        "zh" => "zh",
        "en" => "en",
        _ => system_lang(),
    }
}

/// One of ours, never to be the mic or the headphones (Linux: the virtual mic and 電腦聲音; Windows: VB-CABLE, our
/// virtual mic's way out)
pub fn ours(n: &Node) -> bool {
    if cfg!(windows) {
        // the tests' stand-in for VB-CABLE (audio_win.rs) is ours then, and VB-CABLE their mic's way in
        match std::env::var("STREAM_MIC_TEST_CABLE") {
            Ok(t) => n.desc.contains(&t),
            Err(_) => n.desc.contains("VB-Audio Virtual Cable") || n.desc.starts_with("CABLE "),
        }
    } else {
        n.name.starts_with("stream-mic")
    }
}

/// The named device, or the one the system would pick (highest priority), never one of ours
pub fn pick(nodes: &[Node], class: &str, want: Option<&str>) -> Option<Node> {
    match want {
        // named: any audio device but ours (Linux: a sink as the mic means its monitor)
        Some(w) => nodes.iter().find(|n| n.name == w && !ours(n)).cloned(),
        None => nodes.iter().filter(|n| n.class.starts_with(class) && !ours(n)).max_by_key(|n| n.prio).cloned(),
    }
}

/// What the audio thread keeps between callbacks
pub struct Rt {
    pub shared: Arc<Shared>,
    pub chain: Chain,
    pub gen: u64,
    pub cfg: Cfg,
    pub pc: Ring,
    /// the computer's sound on its way into the virtual mic, delayed to meet the processed voice
    pub pc_delay: Delay,
    pub send: Ring,
    pub mon: Ring,
    /// the readers wait for this much before they start, and the rings are kept near it
    pub target: usize,
    // the Linux outputs' started flags (Windows keeps them per device)
    #[cfg_attr(windows, allow(dead_code))]
    pub send_on: bool,
    #[cfg_attr(windows, allow(dead_code))]
    pub mon_on: bool,
    /// the denoise worker (dnworker.rs)
    pub dn: DnLink,
    pub dnb: Vec<f32>,
    /// samples owed: the worker fell short and silence went out instead; skipped when they arrive, so the delay
    /// stays what the chain was told. Only once the slack cannot grow any more (dn_lat at its most)
    pub dn_debt: usize,
    /// the denoise delay now: the model's and the slack, which grows by the shortfall and 10 ms more each time the
    /// worker falls short, up to dnworker::MARGIN_MAX (one gap, then none: better a little later than gaps)
    pub dn_lat: usize,
    /// silence still to give out while the slack fills up after it grew
    pub dn_pad: usize,
    // scratch, allocated once
    #[cfg_attr(windows, allow(dead_code))]
    pub mic: Vec<f32>,
    pub s: Vec<f32>,
    pub m: Vec<f32>,
    pub pcs: Vec<f32>,
    pub pcm: Vec<f32>,
    pub st: Vec<f32>,
    pub tapb: Vec<f32>,
    /// the raw mic and the computer's sound for auto setup
    pub recb: Vec<f32>,
    /// loudness: the K-weighting's two filters (BS.1770)
    pub kw: [Biquad; 2],
    /// the same for the computer's sound sent, each channel
    pub kwl: [Biquad; 2],
    pub kwr: [Biquad; 2],
    /// the scope's 5 ms: samples so far, mic and voice peaks
    pub hn: usize,
    pub hpk: (f32, f32),
    /// the noise floor reading: the expander's own detector on the mic (100 Hz high-pass, 10 ms peak decay)
    pub gate_hp: Svf,
    pub gate_env: f64,
    pub gate_a: f64,
    /// add the computer's sound to the headphones (Linux: it reaches them only through us; Windows: the system
    /// plays it there itself)
    pub pc_to_monitor: bool,
    /// the computer's sound comes in step with the mic (None: Linux, one graph) or on its own clock, followed by a
    /// resampler that keeps about pc_lead frames waiting (Windows)
    pub pc_asrc: Option<crate::asrc::Asrc>,
    pub pc_lead: usize,
}

/// A direct-form biquad (the K-weighting's published 48 kHz coefficients are in this form)
#[derive(Clone, Copy)]
pub struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    z: [f64; 2],
}

impl Biquad {
    fn tick(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// BS.1770-4's K-weighting at 48 kHz: a high shelf (+4 dB above about 1.5 kHz), then a high-pass at about 38 Hz
const K_WEIGHT: [Biquad; 2] = [
    Biquad { b: [1.53512485958697, -2.69169618940638, 1.19839281085285], a: [-1.69065929318241, 0.73248077421585], z: [0.0; 2] },
    Biquad { b: [1.0, -2.0, 1.0], a: [-1.99004745483398, 0.99007225036621], z: [0.0; 2] },
];

impl Rt {
    fn refresh(&mut self) {
        let g = self.shared.gen.load(Ordering::Acquire);
        if g == self.gen {
            return;
        }
        if let Ok(c) = self.shared.cfg.try_lock() {
            self.cfg = c.clone();
            self.gen = g;
            drop(c);
            self.chain.set_preset(self.cfg.preset.clone());
            self.chain.set_bypass(self.cfg.bypass);
        }
    }

    /// the mic's block: the chain, then both rings
    pub fn mic_block(&mut self, x: &[f32]) {
        let t0 = Instant::now();
        self.refresh();
        let n = x.len();
        let (lb, mv, pv) = (
            // muted while the computer sound is our own voice coming back (loopdet.rs)
            if self.cfg.loopback && !self.shared.loop_mute.load(Ordering::Relaxed) { db(self.cfg.loopback_db) } else { 0.0 },
            if self.cfg.monitor == Monitor::Processed { db(self.cfg.monitor_db) } else { 0.0 },
            if self.pc_to_monitor { db(self.cfg.pc_db) } else { 0.0 },
        );
        // the computer's sound for these frames (none waiting: silence, the normal case without music)
        self.pcs.resize(n * 2, 0.0);
        match self.pc_asrc.as_mut() {
            None => {
                self.pc.pop(&mut self.pcs[..n * 2]);
            }
            // its own clock and cycle (Windows): followed (asrc.rs), about pc_lead frames held back
            Some(a) => {
                let (ok, dropped) = a.pull(&mut self.pc, &mut self.pcs[..n * 2], self.pc_lead);
                let c = &self.shared.count;
                if !ok {
                    c.pc_under.fetch_add(1, Ordering::Relaxed);
                }
                c.pc_drop.fetch_add(dropped as u64, Ordering::Relaxed);
                c.ratio_ppm[2].store((a.drift() * 1e6).round() as u64, Ordering::Relaxed);
            }
        }
        self.pcm.resize(n, 0.0);
        for i in 0..n {
            self.pcm[i] = 0.5 * (self.pcs[2 * i] + self.pcs[2 * i + 1]);
        }
        // the mic to the denoise worker and its result back (passed straight through while denoise is off)
        self.dn.to.push(x);
        self.dn.thread.unpark();
        self.dnb.resize(n, 0.0);
        if self.dn_debt > 0 {
            let w = self.dn.from.waiting();
            if w > n {
                self.dn_debt -= self.dn.from.skip(self.dn_debt.min(w - n));
            }
        }
        let z = self.dn_pad.min(n);
        self.dnb[..z].fill(0.0);
        self.dn_pad -= z;
        let got = z + self.dn.from.pop(&mut self.dnb[z..n]);
        if got < n {
            self.dnb[got..n].fill(0.0);
            self.shared.count.dn_under.fetch_add(1, Ordering::Relaxed);
            let short = n - got;
            if self.dn_lat + short + 480 <= self.dn.model + crate::dnworker::MARGIN_MAX {
                // grow the slack: what was missing is now delay, and 10 ms more of it comes out as silence
                self.dn_lat += short + 480;
                self.dn_pad += 480;
                self.chain.set_external_denoise(Some(self.dn_lat));
                self.shared.count.dn_lat.store(self.dn_lat as u64, Ordering::Relaxed);
                self.shared.count.dn_grew.fetch_add(1, Ordering::Relaxed);
            } else {
                self.dn_debt += short;
            }
        }
        self.s.resize(n, 0.0);
        self.m.resize(n, 0.0);
        self.chain.process_with(x, Some(&self.dnb[..n]), Some(&self.pcm[..n]), &mut self.s[..n], Some(&mut self.m[..n]));
        let mut acc = (0f32, 0f64, 0f32, 0f64, 0f32, 0f64);
        let (mut ksum, mut gsum, mut bent) = (0f64, 0f64, 0u64);
        let (mut pcs_pk, mut pcs_k, mut pcs_ms) = (0f32, 0f64, 0f64);
        let (_, gr_c, gr_l) = self.chain.reduction_db();
        let gr_now = (gr_c + gr_l) as f32;
        self.st.resize(n * 2, 0.0);
        self.tapb.resize(n * 3, 0.0);
        // the voice in the virtual mic is late by the send chain's latency (and by the trip from the headphones to
        // the mic, sync_ms): the computer's sound waits as long, so a song and its backing track stay together
        let dly = (self.chain.send_latency() + (self.cfg.sync_ms.max(0.0) * RATE as f64 / 1000.0).round() as usize).min(self.pc_delay.max());
        for i in 0..n {
            let (l, r) = (self.pcs[2 * i], self.pcs[2 * i + 1]);
            let (dl, dr) = self.pc_delay.step(l, r, dly);
            self.st[2 * i] = guard(self.s[i] + lb * dl);
            self.st[2 * i + 1] = guard(self.s[i] + lb * dr);
            bent += (self.st[2 * i].abs() >= GUARD_KNEE) as u64 + (self.st[2 * i + 1].abs() >= GUARD_KNEE) as u64;
            self.tapb[3 * i] = self.s[i];
            self.tapb[3 * i + 1] = self.pcm[i];
            self.tapb[3 * i + 2] = 0.5 * (self.st[2 * i] + self.st[2 * i + 1]);
            let k0 = self.kw[0].tick(self.s[i] as f64);
            let k = self.kw[1].tick(k0);
            ksum += k * k;
            // the computer's sound as sent (its volume applied, as late as the voice)
            let (sl, sr) = (lb * dl, lb * dr);
            pcs_pk = pcs_pk.max(sl.abs().max(sr.abs()));
            pcs_ms += 0.5 * ((sl as f64).powi(2) + (sr as f64).powi(2));
            let (l0, r0) = (self.kwl[0].tick(sl as f64), self.kwr[0].tick(sr as f64));
            let (kl, kr) = (self.kwl[1].tick(l0), self.kwr[1].tick(r0));
            pcs_k += kl * kl + kr * kr;
            // the scope: every 5 ms, the peaks and the reduction
            self.hpk = (self.hpk.0.max(x[i].abs()), self.hpk.1.max(self.s[i].abs()));
            self.hn += 1;
            if self.hn >= RATE as usize / 200 {
                let dbv = |v: f32| (20.0 * v.max(1e-6).log10()).max(-120.0);
                if self.shared.hist.room() >= 3 {
                    self.shared.hist.push(&[dbv(self.hpk.0), dbv(self.hpk.1), gr_now]);
                }
                self.hn = 0;
                self.hpk = (0.0, 0.0);
            }
            self.gate_env = self.gate_hp.tick(x[i] as f64).abs().max(self.gate_a * self.gate_env);
            gsum += 20.0 * stream_mic_dsp::m::log10(self.gate_env.max(1e-6));
            acc.0 = acc.0.max(x[i].abs());
            acc.1 += (x[i] as f64).powi(2);
            acc.2 = acc.2.max(self.s[i].abs());
            acc.3 += (self.s[i] as f64).powi(2);
            acc.4 = acc.4.max(l.abs().max(r.abs()));
            acc.5 += 0.5 * ((l as f64).powi(2) + (r as f64).powi(2));
        }
        if bent > 0 {
            self.shared.count.send_clip.fetch_add(bent, Ordering::Relaxed);
        }
        self.send.push(&self.st[..n * 2]);
        // to auto setup while it listens: the raw mic and the computer's sound
        if self.shared.rec_on.load(Ordering::Relaxed) && self.shared.rec.room() >= n * 2 {
            self.recb.resize(n * 2, 0.0);
            for i in 0..n {
                self.recb[2 * i] = x[i];
                self.recb[2 * i + 1] = self.pcm[i];
            }
            self.shared.rec.push(&self.recb[..n * 2]);
        }
        // to loop detection, whole blocks only (it keeps the three in step)
        if self.shared.tap.room() >= n * 3 {
            self.shared.tap.push(&self.tapb[..n * 3]);
        }
        for i in 0..n {
            let (l, r) = (self.pcs[2 * i], self.pcs[2 * i + 1]);
            self.st[2 * i] = mv * self.m[i] + pv * l;
            self.st[2 * i + 1] = mv * self.m[i] + pv * r;
        }
        self.mon.push(&self.st[..n * 2]);
        if let Ok(mut a) = self.shared.acc.try_lock() {
            a.in_pk = a.in_pk.max(acc.0);
            a.in_ms += acc.1;
            a.out_pk = a.out_pk.max(acc.2);
            a.out_ms += acc.3;
            a.pc_pk = a.pc_pk.max(acc.4);
            a.pc_ms += acc.5;
            a.k_ms += ksum;
            a.pcs_pk = a.pcs_pk.max(pcs_pk);
            a.pcs_k_ms += pcs_k;
            a.pcs_ms += pcs_ms;
            a.gate_db += gsum;
            let (e, c, l) = self.chain.reduction_db();
            a.gr_exp = a.gr_exp.min(e);
            a.gr_comp = a.gr_comp.min(c);
            a.gr_lim = a.gr_lim.min(l);
            a.n += n as u64;
        }
        let c = &self.shared.count;
        c.send_lat.store(self.chain.send_latency() as u64, Ordering::Relaxed);
        c.mon_lat.store(self.chain.monitor_latency() as u64, Ordering::Relaxed);
        let ((sd, sl), (md, ml)) = (self.chain.send_parts(), self.chain.monitor_parts());
        c.send_dn.store(sd as u64, Ordering::Relaxed);
        c.send_lim.store(sl as u64, Ordering::Relaxed);
        c.mon_dn.store(md as u64, Ordering::Relaxed);
        c.mon_lim.store(ml as u64, Ordering::Relaxed);
        c.blocks.fetch_add(1, Ordering::Relaxed);
        c.quantum.store(n as u64, Ordering::Relaxed);
        let us = t0.elapsed().as_micros() as u64;
        c.worst_us.fetch_max(us, Ordering::Relaxed);
        c.busy_us.fetch_add(us, Ordering::Relaxed);
    }
}

/// Where the send guard starts bending: above the limiter's ceiling (-1 dB), so the voice alone never reaches it
pub const GUARD_KNEE: f32 = 0.9;

/// The virtual mic's last stage: the voice is already limited, but the computer's sound is added after the limiter,
/// and the two together can pass 0 dBFS (a hard clip in OBS or Discord). Straight through up to the knee, then bent
/// smoothly towards 0 dBFS, never reaching it. No look-ahead, so no delay (decided 2026-11-07, step 7).
#[inline]
pub fn guard(x: f32) -> f32 {
    let a = x.abs();
    if a <= GUARD_KNEE {
        x
    } else {
        x.signum() * (GUARD_KNEE + (1.0 - GUARD_KNEE) * ((a - GUARD_KNEE) / (1.0 - GUARD_KNEE)).tanh())
    }
}

impl Rt {
    /// The chain on the saved preset, the denoise worker started, rings for `quantum`-frame cycles
    pub fn new(shared: Arc<Shared>, quantum: usize, pc_to_monitor: bool) -> Result<Rt, String> {
        let cfg = shared.cfg.lock().unwrap().clone();
        let mut chain = Chain::new(cfg.preset.clone())?;
        let dn = dnworker::start(shared.clone(), cfg.preset.denoise_params(), RATE as f64)?;
        // the bypass's dry delay is sized in prepare for the longest the denoise delay may grow to
        chain.set_external_denoise(Some(dn.model + crate::dnworker::MARGIN_MAX));
        chain.prepare(RATE as f64, MAX_BLOCK);
        chain.set_external_denoise(Some(dn.latency));
        shared.count.dn_lat.store(dn.latency as u64, Ordering::Relaxed);
        eprintln!("denoise: hops p99 {:.1} ms, longest {:.1} ms here: slack {:.0} ms", dn.p99_us as f64 / 1000.0, dn.max_us as f64 / 1000.0, (dn.latency - dn.model) as f64 * 1000.0 / RATE as f64);
        chain.set_bypass(cfg.bypass);
        let dn_latency = dn.latency;
        Ok(Rt {
            gen: shared.gen.load(Ordering::Acquire),
            shared: shared.clone(),
            chain,
            cfg,
            pc: Ring::new(RATE as usize / 2, 2),
            pc_delay: Delay::new(RATE as usize),
            send: Ring::new(RATE as usize / 2, 2),
            mon: Ring::new(RATE as usize / 2, 2),
            target: quantum,
            send_on: false,
            mon_on: false,
            dn,
            dnb: Vec::with_capacity(MAX_BLOCK),
            dn_debt: 0,
            dn_lat: dn_latency,
            dn_pad: 0,
            mic: Vec::with_capacity(MAX_BLOCK),
            s: Vec::with_capacity(MAX_BLOCK),
            m: Vec::with_capacity(MAX_BLOCK),
            pcs: Vec::with_capacity(MAX_BLOCK * 2),
            pcm: Vec::with_capacity(MAX_BLOCK),
            st: Vec::with_capacity(MAX_BLOCK * 2),
            tapb: Vec::with_capacity(MAX_BLOCK * 3),
            recb: Vec::with_capacity(MAX_BLOCK * 2),
            kw: K_WEIGHT,
            kwl: K_WEIGHT,
            kwr: K_WEIGHT,
            hn: 0,
            hpk: (0.0, 0.0),
            gate_hp: {
                let mut f = Svf::default();
                f.set_now(Coefs::design(Kind::HighPass, 100.0, std::f64::consts::FRAC_1_SQRT_2, 0.0, RATE as f64));
                f
            },
            gate_env: 0.0,
            gate_a: (-1.0 / (0.010 * RATE as f64)).exp(),
            pc_to_monitor,
            pc_asrc: None,
            pc_lead: 0,
        })
    }
}

/// A mic with more than one channel made mono. Only the first two count: on an interface they are inputs 1 and 2, the
/// rest are other inputs and its own loopback mixes (the Revelator io44 has Line In and Stream Mix A/B there, which
/// carry the computer's sound; PipeWire's own downmix averaged all eight, the mic at 1/8). Of the two, both halves, or
/// only the live one when the other is 60 dB under it (the mic on input 1 and nothing on input 2). Power averaged
/// over about a second of blocks.
#[derive(Default)]
pub struct MicDown {
    e: [f32; 2],
}

impl MicDown {
    /// `x` interleaved with `ch` channels, made mono in place
    pub fn apply(&mut self, x: &mut Vec<f32>, ch: usize) {
        if ch <= 1 || x.is_empty() {
            return;
        }
        let n = x.len() / ch;
        let (mut s0, mut s1) = (0f32, 0f32);
        for i in 0..n {
            s0 += x[ch * i] * x[ch * i];
            s1 += x[ch * i + 1] * x[ch * i + 1];
        }
        self.e[0] = 0.99 * self.e[0] + 0.01 * s0 / n as f32;
        self.e[1] = 0.99 * self.e[1] + 0.01 * s1 / n as f32;
        let (a, b) = mono_weights(self.e);
        for i in 0..n {
            x[i] = a * x[ch * i] + b * x[ch * i + 1];
        }
        x.truncate(n);
    }
}

/// Both halves, or only the live channel when the other is 60 dB under it
pub fn mono_weights(e: [f32; 2]) -> (f32, f32) {
    if e[1] < e[0] * 1e-6 {
        (1.0, 0.0)
    } else if e[0] < e[1] * 1e-6 {
        (0.0, 1.0)
    } else {
        (0.5, 0.5)
    }
}

/// Fill `out` (interleaved, the ring's channels) from `ring`, the way an output callback does. A reader waits until
/// a target's worth is there before it starts; then it is kept near that: never more than a cycle and a target behind
/// (the oldest go: counted, but not the backlog from before a reader came), and short means silence for the rest and
/// waiting again. Returns (had enough, frames dropped).
#[cfg_attr(windows, allow(dead_code))]
pub fn fill(out: &mut [f32], ring: &mut Ring, started: &mut bool, target: usize) -> (bool, usize) {
    let ch = ring.ch;
    let frames = out.len() / ch;
    let mut first = false;
    if !*started && ring.frames() >= target.max(frames) {
        *started = true;
        first = true;
    }
    if !*started {
        out.fill(0.0);
        return (true, 0);
    }
    let d = ring.trim(frames + target);
    let dropped = if first { 0 } else { d };
    let got = ring.pop(&mut out[..frames * ch]);
    if got < frames * ch {
        out[got..].fill(0.0);
        *started = false;
        return (false, dropped);
    }
    (true, dropped)
}

#[cfg(test)]
mod tests {
    #[test]
    fn mono_weights() {
        assert_eq!(super::mono_weights([1e-4, 0.0]), (1.0, 0.0));
        assert_eq!(super::mono_weights([0.0, 1e-4]), (0.0, 1.0));
        assert_eq!(super::mono_weights([1e-4, 5e-5]), (0.5, 0.5));
        assert_eq!(super::mono_weights([0.0, 0.0]), (0.5, 0.5));
    }

    #[test]
    fn eight_channels_take_input_one_only() {
        // the io44 as Linux sees it: the mic on 0, nothing on 1, the computer's sound on the loopback mixes 4..7
        let mut d = super::MicDown::default();
        let mut last = Vec::new();
        for _ in 0..200 {
            let mut x = Vec::new();
            for i in 0..480 {
                let v = (i as f32 * 0.1).sin() * 0.1;
                x.extend_from_slice(&[v, 0.0, 1e-6, 1e-6, 0.5, 0.5, 0.5, 0.5]);
            }
            d.apply(&mut x, 8);
            last = x;
        }
        assert_eq!(last.len(), 480);
        assert!((last[5] - (0.5f32).sin() * 0.1).abs() < 1e-6, "{}", last[5]);
    }

    use super::*;

    #[test]
    fn guard_leaves_the_voice_and_stays_under_full_scale() {
        // the limiter's ceiling (-1 dB) passes untouched
        assert_eq!(guard(0.89), 0.89);
        assert_eq!(guard(-0.5), -0.5);
        // a voice at the ceiling plus music: under 0 dBFS, and still rising (no flat top)
        let (a, b) = (guard(1.09), guard(0.98));
        assert!(a < 1.0 && b < a && b > GUARD_KNEE);
        assert_eq!(guard(-1.09), -a);
        assert!(guard(4.0) <= 1.0);
        // continuous at the knee
        assert!((guard(GUARD_KNEE + 1e-4) - GUARD_KNEE).abs() < 2e-4);
    }
}
