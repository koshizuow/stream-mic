//! What the audio thread and the control side share: the settings (with a generation number, so the audio thread
//! only copies them when they changed, and never waits for the lock), the meters (the audio thread adds, the
//! SSE side takes) and counters.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use stream_mic_chain::Preset;

/// What the headphones get of the voice (routing.md: three ways)
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Monitor {
    /// the processed voice
    Processed,
    /// none of it: the user listens on the interface's own direct monitor (0 ms, unprocessed), so not twice
    Direct,
    /// none of it (speakers: the mic would hear them)
    Off,
}

/// Everything the user sets, saved between runs (settings.json). Levels in dB.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Cfg {
    pub preset: Preset,
    /// the user's changes to each built-in preset, by name: switching back finds them again ("這個模式恢復預設"
    /// drops them)
    pub edited: BTreeMap<String, Preset>,
    /// hear and send the raw voice (the chain's bypass glides over 10 ms); never saved on
    #[serde(skip)]
    pub bypass: bool,
    /// the mic and the headphones, PipeWire node names; None: the system's choice at start
    pub mic: Option<String>,
    pub out: Option<String>,
    pub monitor: Monitor,
    pub monitor_db: f64,
    /// the computer's sound (what plays into "stream-mic 電腦聲音") in the virtual mic
    pub loopback: bool,
    pub loopback_db: f64,
    /// the computer's sound in the headphones: on Linux it reaches them only through us (routing.md)
    pub pc_db: f64,
    /// extra delay of the computer's sound in the virtual mic, ms. It is always delayed by the send chain's own
    /// latency, so a voice and its backing track leave together (user 2026-11-07); this adds the time from the
    /// headphones back to the mic (their output and input buffers), which auto setup can measure
    pub sync_ms: f64,
    /// the page: its layout, language and theme
    pub ui: UiCfg,
}

/// How the page looks, kept with the settings so the app's window and a browser agree
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiCfg {
    /// "a" (hardware: knobs and meters) or "b" (scope: the level over time with the thresholds on it)
    pub layout: String,
    /// "zh" or "en"; empty: the browser's or the system's
    pub lang: String,
    /// "dark", the only one
    pub theme: String,
}

impl Default for UiCfg {
    fn default() -> Self {
        UiCfg { layout: "a".into(), lang: String::new(), theme: "dark".into() }
    }
}

impl Default for Cfg {
    fn default() -> Self {
        Cfg {
            preset: Preset::built_in("talk-directional").expect("built-in preset"),
            edited: BTreeMap::new(),
            bypass: false,
            mic: None,
            out: None,
            monitor: Monitor::Processed,
            monitor_db: 0.0,
            loopback: true,
            loopback_db: 0.0,
            pc_db: 0.0,
            sync_ms: 0.0,
            ui: UiCfg::default(),
        }
    }
}

impl Cfg {
    /// where the settings live: $XDG_CONFIG_HOME/stream-mic/settings.json (tests set it), else on Windows
    /// %APPDATA%\stream-mic\ (a desktop session has no HOME: only ssh sessions set one, so the settings were never
    /// saved when started from the Start menu), else ~/.config/stream-mic/
    pub fn path() -> Option<PathBuf> {
        let home = || std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"));
        let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| {
            if cfg!(windows) {
                std::env::var_os("APPDATA").map(PathBuf::from).or_else(home)
            } else {
                home()
            }
        })?;
        Some(base.join("stream-mic").join("settings.json"))
    }

    /// the saved settings, or None (none yet, or unreadable: said on stderr, the defaults are used)
    pub fn load() -> Option<Cfg> {
        let p = Cfg::path()?;
        let text = std::fs::read_to_string(&p).ok()?;
        let mut v: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("settings {}: {e}; starting from the defaults", p.display());
                return None;
            }
        };
        migrate(&mut v);
        match serde_json::from_value(v) {
            Ok(c) => Some(c),
            Err(e) => {
                eprintln!("settings {}: {e}; starting from the defaults", p.display());
                None
            }
        }
    }

    /// write them (a new file renamed over the old, so a crash never leaves half a file)
    pub fn save(&self) -> std::io::Result<()> {
        let Some(p) = Cfg::path() else { return Ok(()) };
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = p.with_extension("json.new");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).unwrap_or_default())?;
        std::fs::rename(tmp, p)
    }
}

/// Peaks and mean squares since the last take, linear
#[derive(Clone, Copy, Debug, Default)]
pub struct Acc {
    pub in_pk: f32,
    pub in_ms: f64,
    pub out_pk: f32,
    pub out_ms: f64,
    pub pc_pk: f32,
    pub pc_ms: f64,
    /// the voice sent, K-weighted (BS.1770), mean square summed: loudness
    pub k_ms: f64,
    /// the computer's sound as it goes into the virtual mic (after its volume): peak, and K-weighted mean square summed
    /// over both channels (BS.1770 adds the channels' powers)
    pub pcs_pk: f32,
    pub pcs_ms: f64,
    pub pcs_k_ms: f64,
    /// the most each was turning the voice down, dB (<= 0)
    pub gr_exp: f64,
    pub gr_comp: f64,
    pub gr_lim: f64,
    /// the mic's level the way the expander sees it (100 Hz high-pass, 10 ms peak), dB, summed per sample: for the
    /// room's noise floor next to the gate knob
    pub gate_db: f64,
    pub n: u64,
}

#[derive(Default)]
pub struct Counters {
    /// the virtual mic or the headphones asked for audio the chain had not made yet (a click or a gap)
    pub send_under: AtomicU64,
    pub mon_under: AtomicU64,
    /// more waiting than the target: the oldest dropped (clocks drifting apart)
    pub send_drop: AtomicU64,
    pub mon_drop: AtomicU64,
    /// mic blocks processed, and the longest one in microseconds
    pub blocks: AtomicU64,
    pub worst_us: AtomicU64,
    /// the last block's length in frames (the graph's quantum as we see it)
    pub quantum: AtomicU64,
    /// the denoise worker had not caught up when the audio thread needed its samples
    pub dn_under: AtomicU64,
    /// times the denoise slack grew after a shortfall (proc.rs)
    pub dn_grew: AtomicU64,
    /// the denoise worker's longest call, and calls over 5 ms
    pub dn_worst_us: AtomicU64,
    pub dn_slow: AtomicU64,
    /// the computer's sound when it runs on its own clock (Windows): short (silence went in) and dropped
    pub pc_under: AtomicU64,
    pub pc_drop: AtomicU64,
    /// the resamplers' ratios against the mic's clock, x 1e6 (Windows): virtual mic, headphones, computer's sound
    pub ratio_ppm: [AtomicU64; 3],
    /// samples the virtual mic and the headphones lag the mic in the chain (not counting PipeWire's buffers)
    pub send_lat: AtomicU64,
    pub mon_lat: AtomicU64,
    /// the same split: denoise and the limiter, each path
    pub send_dn: AtomicU64,
    pub send_lim: AtomicU64,
    pub mon_dn: AtomicU64,
    pub mon_lim: AtomicU64,
    /// what denoise adds when it is on (the model and the worker's margin), samples
    pub dn_lat: AtomicU64,
    /// microseconds the mic callback has spent in all, for the CPU figure
    pub busy_us: AtomicU64,
    /// samples of the virtual mic the guard bent (the voice plus the computer's sound went over its knee)
    pub send_clip: AtomicU64,
}

pub struct Shared {
    pub cfg: Mutex<Cfg>,
    pub gen: AtomicU64,
    pub acc: Mutex<Acc>,
    pub count: Counters,
    /// the devices chosen at start, for the page
    pub devices: Mutex<serde_json::Value>,
    /// loop detection (loopdet.rs): voice, computer sound and what was sent, interleaved, from the audio thread; and
    /// what it found
    pub tap: crate::dnworker::Spsc,
    pub loop_mute: AtomicBool,
    /// the last correlation x1000 and its delay
    pub loop_corr: AtomicU64,
    pub loop_lag_ms: AtomicU64,
    /// the audio side should start again (the mic or the headphones changed)
    pub restart: AtomicBool,
    /// save the settings when they change (not with --fresh: tests)
    pub persist: bool,
    /// running inside the app window (the page then offers start with the computer), and the page asking the app
    /// to bring its window up (a second start of the app does that, through the page's API)
    pub in_app: AtomicBool,
    pub show: AtomicU64,
    /// auto setup (setup.rs): the raw mic and the computer's sound, interleaved, while `rec_on`; its progress and
    /// result for the page; the page asking it to stop
    pub rec: crate::dnworker::Spsc,
    /// the level every 5 ms for the page's scope: (mic peak dB, voice sent peak dB, compressor and limiter dB), three
    /// floats each, taken by the event stream
    pub hist: crate::dnworker::Spsc,
    pub rec_on: AtomicBool,
    pub setup: Mutex<serde_json::Value>,
    /// the scope's last 10 s (hist drained here), numbered, so each page connected gets all of it from where it was
    pub hist_buf: Mutex<HistBuf>,
    pub setup_cancel: AtomicBool,
    /// a mic stream is there (set on the main loop's thread, read on the data thread: without one the computer
    /// sound's callback runs the chain, on silence)
    pub mic_live: AtomicBool,
}

impl Shared {
    pub fn new(cfg: Cfg, persist: bool) -> Shared {
        Shared {
            cfg: Mutex::new(cfg),
            gen: AtomicU64::new(1),
            acc: Mutex::new(Acc::default()),
            count: Counters::default(),
            devices: Mutex::new(serde_json::json!({})),
            tap: crate::dnworker::Spsc::new(3 * crate::proc::RATE as usize),
            loop_mute: AtomicBool::new(false),
            loop_corr: AtomicU64::new(0),
            loop_lag_ms: AtomicU64::new(0),
            restart: AtomicBool::new(false),
            persist,
            in_app: AtomicBool::new(false),
            show: AtomicU64::new(0),
            rec: crate::dnworker::Spsc::new(2 * 2 * crate::proc::RATE as usize),
            hist: crate::dnworker::Spsc::new(3 * 400),
            rec_on: AtomicBool::new(false),
            setup: Mutex::new(serde_json::json!({ "phase": "idle" })),
            hist_buf: Mutex::new(HistBuf::default()),
            setup_cancel: AtomicBool::new(false),
            mic_live: AtomicBool::new(false),
        }
    }

    pub fn set_cfg(&self, f: impl FnOnce(&mut Cfg)) {
        let mut c = self.cfg.lock().unwrap();
        f(&mut c);
        self.gen.fetch_add(1, Ordering::Release);
    }

    pub fn take_acc(&self) -> Acc {
        std::mem::take(&mut *self.acc.lock().unwrap())
    }
}

/// The scope's points (mic peak, voice peak, reduction; dB), the oldest dropped past 10 s; `next` numbers the next
#[derive(Default)]
pub struct HistBuf {
    pub next: u64,
    pub buf: std::collections::VecDeque<[f32; 3]>,
}

impl HistBuf {
    /// what the audio thread left, in
    pub fn drain(&mut self, from: &crate::dnworker::Spsc) {
        let mut t = [0f32; 300];
        loop {
            let n = from.pop(&mut t) / 3 * 3;
            for p in t[..n].chunks_exact(3) {
                self.buf.push_back([p[0], p[1], p[2]]);
                self.next += 1;
            }
            while self.buf.len() > 2000 {
                self.buf.pop_front();
            }
            if n < t.len() - 2 {
                break;
            }
        }
    }
    /// the points from number `since` on (as many as are still kept), and the number after them
    pub fn since(&self, since: u64) -> (Vec<[f32; 3]>, u64) {
        let first = self.next - self.buf.len() as u64;
        let from = since.max(first);
        (self.buf.iter().skip((from - first) as usize).copied().collect(), self.next)
    }
}

/// A stereo delay line: write a frame, read the one from `d` frames ago
pub struct Delay {
    buf: Vec<f32>,
    w: usize,
}

impl Delay {
    pub fn new(max_frames: usize) -> Delay {
        Delay { buf: vec![0.0; 2 * max_frames], w: 0 }
    }
    pub fn max(&self) -> usize {
        self.buf.len() / 2 - 1
    }
    pub fn step(&mut self, l: f32, r: f32, d: usize) -> (f32, f32) {
        let n = self.buf.len() / 2;
        self.buf[2 * self.w] = l;
        self.buf[2 * self.w + 1] = r;
        let i = (self.w + n - d.min(n - 1)) % n;
        self.w = (self.w + 1) % n;
        (self.buf[2 * i], self.buf[2 * i + 1])
    }
}

/// Frames of `ch` interleaved channels waiting between two callbacks of the same audio thread
pub struct Ring {
    buf: Vec<f32>,
    head: usize,
    len: usize,
    pub ch: usize,
}

impl Ring {
    pub fn new(frames: usize, ch: usize) -> Ring {
        Ring { buf: vec![0.0; frames * ch], head: 0, len: 0, ch }
    }
    /// frames waiting
    pub fn frames(&self) -> usize {
        self.len / self.ch
    }
    pub fn push(&mut self, x: &[f32]) {
        let cap = self.buf.len();
        for &v in x {
            if self.len == cap {
                // full: the oldest frame goes
                self.head = (self.head + self.ch) % cap;
                self.len -= self.ch;
            }
            let i = (self.head + self.len) % cap;
            self.buf[i] = v;
            self.len += 1;
        }
    }
    /// take up to out.len() values; the rest of `out` is zeros. Returns the values taken.
    pub fn pop(&mut self, out: &mut [f32]) -> usize {
        let cap = self.buf.len();
        let n = out.len().min(self.len);
        for (k, o) in out.iter_mut().enumerate() {
            *o = if k < n { self.buf[(self.head + k) % cap] } else { 0.0 };
        }
        self.head = (self.head + n) % cap;
        self.len -= n;
        n
    }
    /// drop the oldest frames so that at most `frames` wait
    pub fn trim(&mut self, frames: usize) -> usize {
        let keep = frames * self.ch;
        if self.len <= keep {
            return 0;
        }
        let d = self.len - keep;
        self.head = (self.head + d) % self.buf.len();
        self.len = keep;
        d / self.ch
    }
}

pub fn db(v: f64) -> f32 {
    10f64.powf(v / 20.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_by_d() {
        let mut d = Delay::new(8);
        let out: Vec<f32> = (0..6).map(|i| d.step(i as f32, -(i as f32), 3).0).collect();
        assert_eq!(out, [0.0, 0.0, 0.0, 0.0, 1.0, 2.0]);
        let mut d = Delay::new(8);
        assert_eq!(d.step(5.0, 6.0, 0), (5.0, 6.0));
    }

    #[test]
    fn ring_keeps_order_drops_oldest() {
        let mut r = Ring::new(4, 2);
        r.push(&[1.0, 1.5, 2.0, 2.5, 3.0, 3.5]);
        assert_eq!(r.frames(), 3);
        let mut o = [0.0; 2];
        r.pop(&mut o);
        assert_eq!(o, [1.0, 1.5]);
        r.push(&[4.0, 4.5, 5.0, 5.5, 6.0, 6.5]); // 2, 3, 4, 5, 6: one too many, 2 goes
        assert_eq!(r.frames(), 4);
        let mut o = [0.0; 10];
        assert_eq!(r.pop(&mut o), 8);
        assert_eq!(o, [3.0, 3.5, 4.0, 4.5, 5.0, 5.5, 6.0, 6.5, 0.0, 0.0]);
        r.push(&[7.0, 7.5, 8.0, 8.5, 9.0, 9.5]);
        assert_eq!(r.trim(1), 2);
        let mut o = [0.0; 2];
        r.pop(&mut o);
        assert_eq!(o, [9.0, 9.5]);
    }
}

/// Settings saved before the low cut became a stage of its own (2026-11-10): eq.hp and eq.hp_base move to
/// lowcut.freq and lowcut.base, in the preset and in every edited one.
fn migrate(v: &mut serde_json::Value) {
    fn preset(p: &mut serde_json::Value) {
        let Some(o) = p.as_object_mut() else { return };
        if o.contains_key("lowcut") {
            return;
        }
        let Some(eq) = o.get_mut("eq").and_then(|e| e.as_object_mut()) else { return };
        let (Some(hp), base) = (eq.remove("hp"), eq.remove("hp_base")) else { return };
        let base = base.unwrap_or_else(|| hp.clone());
        o.insert("lowcut".into(), serde_json::json!({ "freq": hp, "base": base }));
    }
    if let Some(p) = v.get_mut("preset") {
        preset(p);
    }
    if let Some(e) = v.get_mut("edited").and_then(|e| e.as_object_mut()) {
        for p in e.values_mut() {
            preset(p);
        }
    }
}

#[cfg(test)]
mod migrate_tests {
    #[test]
    fn old_low_cut_moves() {
        let mut v = serde_json::json!({ "preset": { "eq": { "hp": 120.0, "hp_base": 100.0, "low": 1.0 } },
            "edited": { "sing": { "eq": { "hp": 90.0 } } } });
        super::migrate(&mut v);
        assert_eq!(v["preset"]["lowcut"], serde_json::json!({ "freq": 120.0, "base": 100.0 }));
        assert!(v["preset"]["eq"].get("hp").is_none());
        assert_eq!(v["edited"]["sing"]["lowcut"]["base"], 90.0);
    }
}
