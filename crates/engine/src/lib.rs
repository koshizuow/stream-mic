//! stream-mic's engine (build-plan.md steps 6 and 9): the audio in and out (Linux: PipeWire, audio.rs; Windows:
//! WASAPI and VB-CABLE, audio_win.rs), the chain between (proc.rs), the local page and its API. Two ways in: the
//! `stream-mic` command line (main.rs, `cli`) and the app window (app/, `start`), which runs the same engine inside
//! its own process.

#[cfg(target_os = "linux")]
mod audio;
#[cfg(windows)]
#[path = "audio_win.rs"]
mod audio;
pub mod autostart;
mod dnworker;
pub mod http;
#[cfg(target_os = "linux")]
mod lat;
mod loopdet;
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) mod asrc;
mod proc;
pub use proc::ui_lang;
mod setup;
pub mod state;

use std::sync::Arc;

pub use state::Shared;

fn arg(args: &[String], k: &str) -> Option<String> {
    args.iter().position(|a| a == k).and_then(|i| args.get(i + 1).cloned())
}

/// The engine running: its settings and meters, and the page's port
pub struct Engine {
    pub shared: Arc<Shared>,
    pub port: u16,
}

/// Settings from settings.json and the options, the page on 127.0.0.1, loop detection, and the audio on its own
/// thread (which changes the mic and the headphones itself, keeping the virtual mic).
///
/// Options: --preset NAME, --mic NODE, --out NODE, --quantum 128, --port 8790, --set knob=v,..., --no-monitor, --fresh.
/// Fails when the port is taken: most likely stream-mic is already running (two would make two virtual mics).
pub fn start(args: &[String]) -> Result<Engine, String> {
    let fresh = args.iter().any(|a| a == "--fresh");
    let mut cfg = if fresh { None } else { state::Cfg::load() }.unwrap_or_default();
    if let Some(p) = arg(args, "--preset") {
        cfg.preset = stream_mic_chain::Preset::built_in(&p).ok_or(format!("no preset {p}"))?;
    }
    if let Some(s) = arg(args, "--set") {
        for kv in s.split(',') {
            if let Some((k, v)) = kv.split_once('=') {
                cfg.preset.set_knob(k.trim(), v.trim().parse().unwrap_or(0.0));
            }
        }
    }
    // on speakers the processed voice in them would ring through the mic: start without it, turn it on in the page
    if args.iter().any(|a| a == "--no-monitor") {
        cfg.monitor = state::Monitor::Off;
    }
    if let Some(m) = arg(args, "--mic") {
        cfg.mic = Some(m);
    }
    if let Some(o) = arg(args, "--out") {
        cfg.out = Some(o);
    }
    let shared = Arc::new(state::Shared::new(cfg, !fresh));
    let port: u16 = arg(args, "--port").and_then(|p| p.parse().ok()).unwrap_or(8790);
    let port = http::start(shared.clone(), port).map_err(|e| format!("port {port}: {e}"))?;
    loopdet::start(shared.clone(), proc::RATE as usize);
    let quantum = arg(args, "--quantum").and_then(|q| q.parse().ok()).unwrap_or(128);
    let s = shared.clone();
    std::thread::Builder::new()
        .name("audio".into())
        .spawn(move || loop {
            let (mic, out) = {
                let c = s.cfg.lock().unwrap();
                (c.mic.clone(), c.out.clone())
            };
            if let Err(e) = audio::run(s.clone(), proc::Opts { mic, out, quantum }) {
                eprintln!("stream-mic: {e}");
                std::process::exit(1);
            }
            eprintln!("the audio stopped: starting it again");
        })
        .map_err(|e| e.to_string())?;
    Ok(Engine { shared, port })
}

/// The command line: `--list` (the audio devices), `--latency PLAY REC [SECONDS] [QUANTUM]` (a round trip), or the
/// engine until stopped
pub fn cli(args: Vec<String>) {
    if args.iter().any(|a| a == "--list") {
        match audio::list() {
            Ok(v) => {
                for n in v {
                    println!("{:12} {:4} {}  ({})", n.class, n.prio, n.name, n.desc);
                }
            }
            Err(e) => eprintln!("list: {e}"),
        }
        return;
    }
    #[cfg(windows)]
    if args.iter().any(|a| a == "--formats") {
        for (n, f) in audio::formats().unwrap_or_default() {
            println!("{:12} {:24} {}", n.class, f, n.desc);
        }
        return;
    }
    // tests on Windows (Linux has pw-play): --play DEVICE FILE.wav SECONDS (looped)
    #[cfg(windows)]
    if let Some(i) = args.iter().position(|a| a == "--play") {
        let (Some(d), Some(f), Some(t)) = (args.get(i + 1), args.get(i + 2), args.get(i + 3).and_then(|t| t.parse().ok())) else {
            eprintln!("usage: stream-mic --play DEVICE FILE.wav SECONDS");
            std::process::exit(2)
        };
        if let Err(e) = audio::play_wav(d, f, t) {
            eprintln!("play: {e}");
            std::process::exit(1);
        }
        return;
    }
    // tests on Windows (Linux has pw-record): --capture DEVICE SECONDS OUT.wav
    #[cfg(windows)]
    if let Some(i) = args.iter().position(|a| a == "--capture") {
        let (Some(d), Some(t), Some(o)) = (args.get(i + 1), args.get(i + 2).and_then(|t| t.parse().ok()), args.get(i + 3)) else {
            eprintln!("usage: stream-mic --capture DEVICE SECONDS OUT.wav");
            std::process::exit(2)
        };
        if let Err(e) = audio::capture_to_wav(d, t, o) {
            eprintln!("capture: {e}");
            std::process::exit(1);
        }
        return;
    }
    #[cfg(target_os = "linux")]
    if let Some(i) = args.iter().position(|a| a == "--latency") {
        let (Some(play), Some(rec)) = (args.get(i + 1), args.get(i + 2)) else {
            eprintln!("usage: stream-mic --latency PLAY_NODE REC_NODE [SECONDS] [QUANTUM]");
            std::process::exit(2)
        };
        let secs = args.get(i + 3).and_then(|s| s.parse().ok()).unwrap_or(10.0);
        let q = args.get(i + 4).and_then(|s| s.parse().ok()).unwrap_or(128);
        if let Err(e) = lat::run(play, rec, secs, q) {
            eprintln!("latency test: {e}");
            std::process::exit(1);
        }
        return;
    }
    match start(&args) {
        Ok(e) => eprintln!("control: http://127.0.0.1:{}/", e.port),
        Err(e) => {
            eprintln!("stream-mic: {e}");
            std::process::exit(1);
        }
    }
    loop {
        std::thread::park();
    }
}
