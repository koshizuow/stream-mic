//! stream-mic-cli IN.wav OUT.wav [--block N] [--denoise limit=24] [--eq k=v,..] [--deess k=v,..] [--comp k=v,..] [--exp k=v,..] [--lim k=v,..]
//!
//! --denoise limit (dB, 0 = off; 48 kHz only).
//! --preset NAME|FILE runs the whole chain instead (talk-omni, talk-directional, sing, or a .toml): --set knob=v,..
//! turns knobs (denoise body space sib), --music M.wav, --monitor MON.wav writes the monitor path too, --bypass,
//! --realtime paces the blocks as audio arrives. --show-preset NAME prints a preset as TOML.
//! --autoset NOISE.wav VOICE.wav [--preset P] [--target -18] [--write OUT.toml]: auto setup (build-plan step 8).
//! Stages run in the order given; --verb (t60 pre lowcut damp mix mixdry follow er erms) comes last, with
//! --music M.wav as the backing track it follows (without it, mix applies all the time) and
//! --stereo for a two-channel output. Keys: --eq hp low mid midf pres air; --deess thr freq q ratio max att
//! rel; --comp thr ratio knee att rel makeup
//! schp; --exp thr ratio range knee att hold rel det schp; --lim ceil rel look hold tp.
//!
//! Reads the first channel of IN (any format hound reads), runs the stages given, writes 32-bit
//! float mono OUT. Prints each stage's latency and the processing speed.

use std::process::ExitCode;
use std::time::Instant;

use stream_mic_chain::{Chain, Preset};
use stream_mic_denoise::{Denoise, DenoiseParams};
use stream_mic_dsp::deesser::{DeessParams, Deesser};
use stream_mic_dsp::dynamics::{CompParams, Compressor, ExpParams, Expander};
use stream_mic_dsp::eq::{Eq, EqParams};
use stream_mic_dsp::limiter::{LimParams, Limiter};
use stream_mic_dsp::reverb::{Reverb, VerbParams};
use stream_mic_dsp::Stage;

fn usage() -> ExitCode {
    eprintln!("usage: stream-mic-cli IN.wav OUT.wav [--block N] [--denoise limit=24] [--eq k=v,..] [--deess k=v,..] [--comp k=v,..] [--exp k=v,..] [--lim k=v,..] [--verb k=v,.. [--music M.wav] [--stereo]]");
    ExitCode::from(2)
}

fn kv(s: &str) -> Result<Vec<(String, f64)>, String> {
    s.split(',')
        .filter(|kv| !kv.is_empty())
        .map(|kv| {
            let (k, v) = kv.split_once('=').ok_or_else(|| format!("bad item {kv}"))?;
            Ok((k.to_string(), v.parse().map_err(|_| format!("bad number in {kv}"))?))
        })
        .collect()
}

fn parse_denoise(s: &str) -> Result<DenoiseParams, String> {
    let mut p = DenoiseParams::default();
    for (k, v) in kv(s)? {
        match k.as_str() {
            "limit" => p.limit_db = v,
            _ => return Err(format!("unknown --denoise key {k}")),
        }
    }
    Ok(p)
}

fn parse_eq(s: &str) -> Result<EqParams, String> {
    let mut p = EqParams::default();
    for (k, v) in kv(s)? {
        match k.as_str() {
            "hp" => p.hp_hz = v,
            "low" => p.low_db = v,
            "mid" => p.mid_db = v,
            "midf" => p.mid_hz = v,
            "pres" => p.pres_db = v,
            "air" => p.air_db = v,
            _ => return Err(format!("unknown --eq key {k}")),
        }
    }
    Ok(p)
}

fn parse_comp(s: &str) -> Result<CompParams, String> {
    let mut p = CompParams::default();
    for (k, v) in kv(s)? {
        match k.as_str() {
            "thr" => p.thr_db = v,
            "ratio" => p.ratio = v,
            "knee" => p.knee_db = v,
            "att" => p.att_ms = v,
            "rel" => p.rel_ms = v,
            "makeup" => p.makeup_db = v,
            "schp" => p.sc_hp_hz = v,
            _ => return Err(format!("unknown --comp key {k}")),
        }
    }
    Ok(p)
}

fn parse_exp(s: &str) -> Result<ExpParams, String> {
    let mut p = ExpParams::default();
    for (k, v) in kv(s)? {
        match k.as_str() {
            "thr" => p.thr_db = v,
            "ratio" => p.ratio = v,
            "range" => p.range_db = v,
            "knee" => p.knee_db = v,
            "att" => p.att_ms = v,
            "hold" => p.hold_ms = v,
            "rel" => p.rel_ms = v,
            "det" => p.det_ms = v,
            "schp" => p.sc_hp_hz = v,
            _ => return Err(format!("unknown --exp key {k}")),
        }
    }
    Ok(p)
}

fn parse_lim(s: &str) -> Result<LimParams, String> {
    let mut p = LimParams::default();
    for (k, v) in kv(s)? {
        match k.as_str() {
            "ceil" => p.ceil_db = v,
            "rel" => p.rel_ms = v,
            "look" => p.look_ms = v,
            "hold" => p.hold_ms = v,
            "tp" => p.true_peak = v != 0.0,
            _ => return Err(format!("unknown --lim key {k}")),
        }
    }
    Ok(p)
}

fn parse_deess(s: &str) -> Result<DeessParams, String> {
    let mut p = DeessParams::default();
    for (k, v) in kv(s)? {
        match k.as_str() {
            "thr" => p.thr_db = v,
            "freq" => p.freq_hz = v,
            "q" => p.q = v,
            "ratio" => p.ratio = v,
            "max" => p.max_db = v,
            "att" => p.att_ms = v,
            "rel" => p.rel_ms = v,
            _ => return Err(format!("unknown --deess key {k}")),
        }
    }
    Ok(p)
}

fn parse_verb(s: &str) -> Result<VerbParams, String> {
    let mut p = VerbParams::default();
    for (k, v) in kv(s)? {
        match k.as_str() {
            "t60" => p.t60_s = v,
            "pre" => p.pre_ms = v,
            "lowcut" => p.lowcut_hz = v,
            "damp" => p.damp_hz = v,
            "mix" => p.mix = v,
            "mixdry" => p.mix_dry = v,
            "follow" => p.follow_s = v,
            "er" => p.er = v,
            "erms" => p.er_ms = v,
            _ => return Err(format!("unknown --verb key {k}")),
        }
    }
    Ok(p)
}

fn read_wav(path: &str) -> Result<(Vec<f32>, u32), String> {
    let mut r = hound::WavReader::open(path).map_err(|e| format!("{path}: {e}"))?;
    let spec = r.spec();
    let ch = spec.channels as usize;
    let all: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().collect::<Result<_, _>>(),
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1u64 << (spec.bits_per_sample - 1)) as f32;
            r.samples::<i32>().map(|s| s.map(|v| v as f32 * scale)).collect::<Result<_, _>>()
        }
    }
    .map_err(|e| format!("{path}: {e}"))?;
    Ok((all.into_iter().step_by(ch).collect(), spec.sample_rate))
}

fn write_wav(path: &str, x: &[f32], fs: u32) -> Result<(), String> {
    write_wav_ch(path, x, fs, 1)
}

fn write_wav_ch(path: &str, x: &[f32], fs: u32, channels: u16) -> Result<(), String> {
    let spec = hound::WavSpec { channels, sample_rate: fs, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
    let mut w = hound::WavWriter::create(path, spec).map_err(|e| format!("{path}: {e}"))?;
    for &s in x {
        w.write_sample(s).map_err(|e| format!("{path}: {e}"))?;
    }
    w.finalize().map_err(|e| format!("{path}: {e}"))
}

/// --autoset NOISE.wav VOICE.wav [--preset NAME|FILE] [--target LUFS] [--write OUT.toml]: auto setup from a recording
/// of the room alone and one of talking (48 kHz); prints what it found, one key=value per line
fn run_autoset(noise: &str, voice: &str, preset: &str, target: f64, write: Option<&str>) -> Result<(), String> {
    let (n, fs1) = read_wav(noise)?;
    let (v, fs2) = read_wav(voice)?;
    if fs1 != 48000 || fs2 != 48000 {
        return Err("--autoset needs 48 kHz".into());
    }
    let base = load_preset(preset)?;
    let mut chain = Chain::new(base.clone())?;
    let r = stream_mic_chain::autoset::autoset(&mut chain, &base, &n, &v, target);
    println!("floor_db={:.1}\nvoice_db={:.1}\npeak_db={:.1}", r.floor_db, r.voice_db, r.peak_db);
    println!("gate={}\ncomp_thr={}\ncomp_gr_db={:.1}\nmakeup={}", r.preset.exp.thr, r.preset.comp.thr, r.comp_gr_db, r.preset.comp.makeup);
    println!("lufs={:.2}\nst10={:.2}\nst50={:.2}\nst90={:.2}", r.lufs, r.st[0], r.st[1], r.st[2]);
    println!("warnings={:?}", r.warnings);
    if let Some(w) = write {
        std::fs::write(w, r.preset.to_toml()).map_err(|e| format!("{w}: {e}"))?;
    }
    Ok(())
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut pos = Vec::new();
    let mut block = 128usize;
    let mut stages: Vec<(&'static str, Box<dyn Stage>)> = Vec::new();
    let mut verb: Option<Reverb> = None;
    let mut music: Option<String> = None;
    let mut stereo = false;
    let mut preset_arg: Option<String> = None;
    let mut knob_sets: Vec<(String, f64)> = Vec::new();
    let mut monitor_out: Option<String> = None;
    let mut bypass = false;
    let mut realtime = false;
    let mut autoset = false;
    let mut write: Option<String> = None;
    let mut target = stream_mic_chain::autoset::TARGET;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--block" => {
                i += 1;
                block = args.get(i).and_then(|v| v.parse().ok()).filter(|&b| b > 0).ok_or("bad --block")?;
            }
            "--denoise" | "--eq" | "--deess" | "--comp" | "--exp" | "--lim" => {
                if verb.is_some() {
                    return Err("--verb must be the last stage".into());
                }
                let opt = args[i].clone();
                i += 1;
                let v = args.get(i).map(String::as_str).unwrap_or("");
                stages.push(match opt.as_str() {
                    "--denoise" => ("denoise", Box::new(Denoise::new(parse_denoise(v)?)?) as Box<dyn Stage>),
                    "--eq" => ("eq", Box::new(Eq::new(parse_eq(v)?))),
                    "--deess" => ("deess", Box::new(Deesser::new(parse_deess(v)?))),
                    "--comp" => ("comp", Box::new(Compressor::new(parse_comp(v)?))),
                    "--exp" => ("exp", Box::new(Expander::new(parse_exp(v)?))),
                    _ => ("lim", Box::new(Limiter::new(parse_lim(v)?))),
                });
            }
            "--verb" => {
                i += 1;
                verb = Some(Reverb::new(parse_verb(args.get(i).map(String::as_str).unwrap_or(""))?));
            }
            "--music" => {
                i += 1;
                music = Some(args.get(i).ok_or("--music needs a file")?.clone());
            }
            "--stereo" => stereo = true,
            "--preset" => {
                i += 1;
                preset_arg = Some(args.get(i).ok_or("--preset needs a name or a file")?.clone());
            }
            "--set" => {
                i += 1;
                for (k, v) in kv(args.get(i).map(String::as_str).unwrap_or(""))? {
                    knob_sets.push((k, v));
                }
            }
            "--monitor" => {
                i += 1;
                monitor_out = Some(args.get(i).ok_or("--monitor needs a file")?.clone());
            }
            "--bypass" => bypass = true,
            "--realtime" => realtime = true,
            "--autoset" => autoset = true,
            "--write" => {
                i += 1;
                write = Some(args.get(i).ok_or("--write needs a file")?.clone());
            }
            "--target" => {
                i += 1;
                target = args.get(i).and_then(|v| v.parse().ok()).ok_or("bad --target")?;
            }
            "--show-preset" => {
                i += 1;
                let p = load_preset(args.get(i).ok_or("--show-preset needs a name or a file")?)?;
                print!("{}", p.to_toml());
                return Ok(());
            }
            a if a.starts_with("--") => return Err(format!("unknown option {a}")),
            a => pos.push(a.to_string()),
        }
        i += 1;
    }
    if pos.len() != 2 {
        return Err(String::new());
    }
    if autoset {
        return run_autoset(&pos[0], &pos[1], preset_arg.as_deref().unwrap_or("talk-directional"), target, write.as_deref());
    }
    if let Some(name) = &preset_arg {
        if !stages.is_empty() || verb.is_some() {
            return Err("--preset runs the whole chain: no single stages with it".into());
        }
        return run_chain(&pos[0], &pos[1], name, &knob_sets, music.as_deref(), monitor_out.as_deref(), block, bypass, realtime);
    }
    let (mut x, fs) = read_wav(&pos[0])?;
    for (_, s) in stages.iter_mut() {
        s.prepare(fs as f64, block);
    }
    let music = match &music {
        Some(m) => Some(read_wav(m)?.0),
        None => None,
    };
    if let Some(v) = verb.as_mut() {
        v.prepare(fs as f64, block);
        if music.is_none() {
            v.without_music();
        }
    }
    let (mut left, mut right) = (vec![0f32; x.len()], vec![0f32; x.len()]);
    let t = Instant::now();
    for (k, chunk) in x.chunks_mut(block).enumerate() {
        for (_, s) in stages.iter_mut() {
            s.process(chunk);
        }
        if let Some(v) = verb.as_mut() {
            if let Some(m) = &music {
                let a = (k * block).min(m.len());
                let b = (k * block + chunk.len()).min(m.len());
                v.set_music(&m[a..b]);
            }
            if stereo {
                let r = k * block..k * block + chunk.len();
                v.process_stereo(chunk, &mut left[r.clone()], &mut right[r]);
            } else {
                v.process(chunk);
            }
        }
    }
    let secs = t.elapsed().as_secs_f64();
    let audio = x.len() as f64 / fs as f64;
    for (name, s) in &stages {
        eprintln!("{name}: latency {} samples", s.latency());
    }
    if let Some(v) = &verb {
        eprintln!("verb: latency {} samples", v.latency());
    }
    eprintln!("{:.1} s of audio in {:.3} s ({:.2} % of one core)", audio, secs, 100.0 * secs / audio.max(1e-9));
    if verb.is_some() && stereo {
        let lr: Vec<f32> = left.iter().zip(&right).flat_map(|(l, r)| [*l, *r]).collect();
        write_wav_ch(&pos[1], &lr, fs, 2)
    } else {
        write_wav(&pos[1], &x, fs)
    }
}

fn load_preset(name: &str) -> Result<Preset, String> {
    match Preset::built_in(name) {
        Some(p) => Ok(p),
        None => Preset::parse(&std::fs::read_to_string(name).map_err(|e| format!("{name}: {e}"))?),
    }
}

/// The whole chain with a preset: send (and monitor) out, a per-stage table of CPU and the latencies
#[allow(clippy::too_many_arguments)]
fn run_chain(inp: &str, out: &str, name: &str, sets: &[(String, f64)], music: Option<&str>, monitor: Option<&str>, block: usize, bypass: bool, realtime: bool) -> Result<(), String> {
    let mut p = load_preset(name)?;
    for (k, v) in sets {
        p.set_knob(k, *v);
    }
    let (x, fs) = read_wav(inp)?;
    let m = match music {
        Some(f) => Some(read_wav(f)?.0),
        None => None,
    };
    let mut c = Chain::new(p)?;
    c.prepare(fs as f64, block);
    c.set_bypass(bypass);
    c.times = Some([0.0; 7]);
    let (mut s, mut mon) = (vec![0f32; x.len()], vec![0f32; x.len()]);
    let period = std::time::Duration::from_secs_f64(block as f64 / fs as f64);
    let start = Instant::now();
    let mut worst = 0f64;
    let mut k = 0;
    let mut i = 0;
    while i < x.len() {
        let j = (i + block).min(x.len());
        let mm = m.as_ref().map(|m| &m[i.min(m.len())..j.min(m.len())]);
        let t = Instant::now();
        c.process(&x[i..j], mm, &mut s[i..j], if monitor.is_some() { Some(&mut mon[i..j]) } else { None });
        worst = worst.max(t.elapsed().as_secs_f64());
        k += 1;
        if realtime {
            // as fast as audio arrives, like the engine will (the CPU clocks down and caches go cold between blocks)
            let due = start + period * k as u32;
            let now = Instant::now();
            if due > now {
                std::thread::sleep(due - now);
            }
        }
        i = j;
    }
    let audio = x.len() as f64 / fs as f64;
    let times = c.times.unwrap_or_default();
    let on = &c.preset().on;
    let flags = [on.denoise, on.eq, on.deess, on.exp, on.comp, on.verb, on.lim];
    eprintln!("preset {} ({}), block {block}{}", c.preset().name, c.preset().label, if realtime { ", real-time pace" } else { "" });
    eprintln!("{:8} {:>7} {:>7}", "stage", "own %", "cum %");
    let mut cum = 0.0;
    for (n, (name, t)) in stream_mic_chain::STAGE_NAMES.iter().zip(times).enumerate() {
        if !flags[n] {
            continue;
        }
        cum += t;
        eprintln!("{:8} {:7.2} {:7.2}", name, 100.0 * t / audio, 100.0 * cum / audio);
    }
    eprintln!("worst block {:.3} ms (deadline {:.3} ms)", worst * 1e3, period.as_secs_f64() * 1e3);
    eprintln!("latency: send {} samples ({:.2} ms), monitor {} samples ({:.2} ms)", c.send_latency(), c.send_latency() as f64 / fs as f64 * 1e3, c.monitor_latency(), c.monitor_latency() as f64 / fs as f64 * 1e3);
    write_wav(out, &s, fs)?;
    if let Some(mo) = monitor {
        write_wav(mo, &mon, fs)?;
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.is_empty() => usage(),
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
