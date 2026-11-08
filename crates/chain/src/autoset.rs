//! Auto setup (build-plan.md step 8, design/dsp/loudness.md): from 3 seconds of the room alone and about 10 seconds
//! of the user talking the way they stream, set the preset's
//!
//! - expander threshold (the gate knob): 10 dB above the room's noise floor (singing 8), as the expander's own
//!   detector sees it (100 Hz high-pass, 10 ms peak), and at least 20 dB under the voice;
//! - compressor threshold: where it turns the voice down by a few dB while it talks (4 dB, singing 3);
//! - compressor make-up gain: the processed voice at the target loudness (BS.1770, -18 LUFS for talk, gated like the
//!   integrated loudness so the pauses do not count), the limiter at -1 dBTP as before.
//!
//! The level found is measured on the chain's own output, run on the recording (denoise and the reverb off while
//! measuring: they hardly move the voice's level and denoise is slow). Loudness counts both channels of the virtual
//! mic (the voice is in both), as OBS and the page's meter do.

use stream_mic_dsp::svf::{Coefs, Kind, Svf};

use crate::{Chain, Preset};

const FS: f64 = 48000.0;
/// 100 ms: the loudness blocks
const BLOCK: usize = 4800;
/// 10 ms: the level frames and the chain's block
const FRAME: usize = 480;

#[derive(Clone, Copy)]
struct Biquad {
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

/// BS.1770-4's K-weighting at 48 kHz
const K_WEIGHT: [Biquad; 2] = [
    Biquad { b: [1.53512485958697, -2.69169618940638, 1.19839281085285], a: [-1.69065929318241, 0.73248077421585], z: [0.0; 2] },
    Biquad { b: [1.0, -2.0, 1.0], a: [-1.99004745483398, 0.99007225036621], z: [0.0; 2] },
];

/// K-weighted mean square of each whole 100 ms block
fn k_blocks(x: &[f32]) -> Vec<f64> {
    let mut kw = K_WEIGHT;
    x.chunks_exact(BLOCK)
        .map(|c| {
            c.iter()
                .map(|&v| {
                    let k0 = kw[0].tick(v as f64);
                    let k = kw[1].tick(k0);
                    k * k
                })
                .sum::<f64>()
                / BLOCK as f64
        })
        .collect()
}

/// LUFS of a mean square, the mono voice counted in both channels
fn lufs(ms: f64) -> f64 {
    if ms <= 0.0 {
        -120.0
    } else {
        -0.691 + 10.0 * (2.0 * ms).log10()
    }
}

/// Mean squares over `n` blocks, sliding by one block
fn windows(b: &[f64], n: usize) -> Vec<f64> {
    if b.len() < n {
        return Vec::new();
    }
    (0..=b.len() - n).map(|i| b[i..i + n].iter().sum::<f64>() / n as f64).collect()
}

/// Integrated loudness (BS.1770: 400 ms blocks overlapping 75 %, gated at -70 LUFS and 10 LU under the mean)
pub fn integrated(x: &[f32]) -> f64 {
    let m = windows(&k_blocks(x), 4);
    let abs: Vec<f64> = m.iter().copied().filter(|&v| lufs(v) > -70.0).collect();
    if abs.is_empty() {
        return -120.0;
    }
    let rel = lufs(abs.iter().sum::<f64>() / abs.len() as f64) - 10.0;
    let g: Vec<f64> = abs.into_iter().filter(|&v| lufs(v) > rel).collect();
    lufs(g.iter().sum::<f64>() / g.len() as f64)
}

/// The short-term loudness (3 s windows, every 100 ms) where the voice is: windows within 10 LU of `integrated`
pub fn short_term(x: &[f32], integrated: f64) -> Vec<f64> {
    windows(&k_blocks(x), 30).into_iter().map(lufs).filter(|&l| l > integrated - 10.0).collect()
}

fn pct(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    s[((s.len() - 1) as f64 * p).round() as usize]
}

/// The level the expander's detector sees (100 Hz high-pass, 10 ms peak decay), dB, the mean of each 10 ms frame
fn gate_levels(x: &[f32]) -> Vec<f64> {
    let mut hp = Svf::default();
    hp.set_now(Coefs::design(Kind::HighPass, 100.0, std::f64::consts::FRAC_1_SQRT_2, 0.0, FS));
    let a = (-1.0 / (0.010 * FS)).exp();
    let mut env = 0f64;
    x.chunks_exact(FRAME)
        .map(|c| {
            c.iter()
                .map(|&v| {
                    env = hp.tick(v as f64).abs().max(a * env);
                    20.0 * env.max(1e-6).log10()
                })
                .sum::<f64>()
                / FRAME as f64
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub enum Warning {
    /// the voice is hardly louder than the room (dB between them): turn denoise up, or move closer to the mic
    Noisy(f64),
    /// the make-up needed is more than this many dB: raise the interface's gain (the noise comes up with the voice)
    TooQuiet(f64),
    /// the recording reached this peak, dBFS: lower the interface's gain before it clips
    TooLoud(f64),
    /// no voice in the talking part
    NoVoice,
}

#[derive(Clone, Debug)]
pub struct Report {
    /// the preset with the three values set (the gate knob moved with its threshold)
    pub preset: Preset,
    /// the room's level (95th percentile) and the voice's (90th), dB as the expander sees them
    pub floor_db: f64,
    pub voice_db: f64,
    /// the recording's sample peak, dBFS
    pub peak_db: f64,
    /// the compressor's reduction while the voice is there (median), dB (<= 0)
    pub comp_gr_db: f64,
    /// the result: integrated loudness and the short-term spread (10th, 50th, 90th percentile), LUFS
    pub lufs: f64,
    pub st: [f64; 3],
    pub warnings: Vec<Warning>,
}

/// The default target for a preset: -18 LUFS (AES TD1008, speech)
pub const TARGET: f64 = -18.0;

/// Run the chain over `x` with preset `p`: the send output, lined up with the input, and the compressor's reduction
/// per 10 ms frame
fn run(chain: &mut Chain, p: &Preset, x: &[f32]) -> (Vec<f32>, Vec<f64>) {
    chain.set_preset(p.clone());
    chain.prepare(FS, FRAME);
    let mut out = vec![0f32; x.len()];
    let mut gr = Vec::with_capacity(x.len() / FRAME + 1);
    for (i, o) in x.chunks(FRAME).zip(out.chunks_mut(FRAME)) {
        chain.process(i, None, o, None);
        gr.push(chain.reduction_db().1);
    }
    let lat = chain.send_latency().min(out.len());
    out.drain(..lat);
    out.resize(x.len(), 0.0);
    (out, gr)
}

/// Set `base` from `noise` (the room alone, about 3 s) and `voice` (talking, about 10 s), both the raw mic at 48 kHz.
/// `chain` is any chain (its model is reused; its preset and state are changed).
pub fn autoset(chain: &mut Chain, base: &Preset, noise: &[f32], voice: &[f32], target: f64) -> Report {
    let mut warnings = Vec::new();
    let nl = gate_levels(noise);
    let vl = gate_levels(voice);
    let floor = pct(&nl, 0.95);
    let voice_db = pct(&vl, 0.90);
    let peak = voice.iter().fold(0f32, |m, v| m.max(v.abs()));
    let peak_db = 20.0 * (peak.max(1e-6) as f64).log10();
    // no voice: the talking part hardly louder than the room (loudness, so a steady noise and a voice compare fairly);
    // noisy: the expander cannot tell the pauses from the voice well (its own detector's levels)
    if integrated(voice) - integrated(noise) < 6.0 {
        warnings.push(Warning::NoVoice);
    } else if voice_db - floor < 20.0 {
        warnings.push(Warning::Noisy(voice_db - floor));
    }
    if peak_db > -1.0 {
        warnings.push(Warning::TooLoud(peak_db));
    }

    // the gate: over the room by enough that the pauses get most of the range, but at least 20 dB under the voice's
    // loud parts, or quiet words would be turned down. Talk 10 dB: at 6 the knee (10 dB wide) left the room half
    // inside it and the pauses came down only 4 dB, which the make-up then raised over the room as it was (AT9931PC
    // through the speaker, 2026-11-08: pauses -4.0 dB at +6, -6.5 at +8, -9.4 at +10, -11.7 at +12, the voice within
    // 30 dB of its peak untouched up to +10). Singing 8 (measurements.md: at +14 the sung parts are turned down)
    let mut p = base.clone();
    let over = if base.follow_music { 8.0 } else { 10.0 };
    let thr = (floor + over).min(voice_db - 20.0).clamp(-80.0, -20.0);
    p.set_knob("gate", (thr * 2.0).round() / 2.0);

    // measured without denoise and the reverb; the room first, so every detector has settled when the voice comes
    let mut m = p.clone();
    m.on.denoise = false;
    m.on.verb = false;
    let mut x = noise.to_vec();
    x.extend_from_slice(voice);
    let start = noise.len() / FRAME;
    // frames where the voice is (within 20 dB of its loud parts)
    let talking: Vec<bool> = vl.iter().map(|&l| l > voice_db - 20.0).collect();
    let want_gr = if base.follow_music { -3.0 } else { -4.0 };
    let gr_at = |chain: &mut Chain, m: &mut Preset, thr: f64| -> f64 {
        m.comp.thr = thr;
        m.comp.makeup = 0.0;
        let (_, gr) = run(chain, m, &x);
        let g: Vec<f64> = gr[start..].iter().zip(&talking).filter(|(_, &t)| t).map(|(&g, _)| g).collect();
        pct(&g, 0.5)
    };
    // the compressor's threshold: its reduction falls as the threshold rises
    let (mut lo, mut hi) = (-60.0f64, -6.0f64);
    let mut comp_gr = 0.0;
    if !warnings.contains(&Warning::NoVoice) {
        for _ in 0..12 {
            let mid = 0.5 * (lo + hi);
            let g = gr_at(chain, &mut m, mid);
            if g < want_gr {
                lo = mid;
            } else {
                hi = mid;
            }
            if hi - lo < 0.25 {
                break;
            }
        }
        let thr = (0.5 * (lo + hi) * 2.0).round() / 2.0;
        comp_gr = gr_at(chain, &mut m, thr);
    }
    p.comp.thr = m.comp.thr;

    // the make-up: the output's loudness to the target, a few rounds (the limiter is not linear)
    let mut makeup = 0.0;
    let mut out_l = -120.0;
    let mut out = Vec::new();
    for _ in 0..4 {
        m.comp.makeup = makeup;
        out = run(chain, &m, &x).0.split_off(noise.len());
        out_l = integrated(&out);
        let step = target - out_l;
        if step.abs() < 0.2 {
            break;
        }
        makeup = (makeup + step).clamp(-12.0, 36.0);
    }
    let makeup = (makeup * 2.0).round() / 2.0;
    if makeup > 24.0 {
        warnings.push(Warning::TooQuiet(makeup));
    }
    p.comp.makeup = makeup;
    let st = short_term(&out, out_l);
    Report {
        preset: p,
        floor_db: floor,
        voice_db,
        peak_db,
        comp_gr_db: comp_gr,
        lufs: out_l,
        st: [pct(&st, 0.1), pct(&st, 0.5), pct(&st, 0.9)],
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_scale_sine_at_1khz_reads_about_its_level() {
        // BS.1770: a 0 dBFS 1 kHz sine in one channel reads -3.01 LUFS; ours counts it twice: 0
        let x: Vec<f32> = (0..48000 * 3).map(|i| (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / FS).sin() as f32).collect();
        let l = integrated(&x);
        assert!((l - 0.0).abs() < 0.2, "{l}");
    }

    fn noise(n: usize, seed: u32, amp: f32) -> Vec<f32> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                amp * (s as f32 / u32::MAX as f32 * 2.0 - 1.0)
            })
            .collect()
    }

    /// bursts of a voice-like sound (a 150 Hz buzz with harmonics), syllables of 200 ms with gaps, at `level`
    fn talk(secs: f64, level: f32) -> Vec<f32> {
        let n = (secs * FS) as usize;
        (0..n)
            .map(|i| {
                let t = i as f64 / FS;
                let syl = (t * 4.0).fract() < 0.6 && (t * 0.5).fract() < 0.85;
                let env = if syl { (0.6 + 0.4 * (t * 1.3).sin()) as f32 } else { 0.0 };
                let v: f64 = (1..8).map(|h| (2.0 * std::f64::consts::PI * 150.0 * h as f64 * t).sin() / h as f64).sum();
                level * env * v as f32 * 0.5
            })
            .collect()
    }

    #[test]
    fn reaches_the_target_whatever_the_mic_level() {
        let mut chain = Chain::new(Preset::built_in("talk-directional").unwrap()).unwrap();
        let base = Preset::built_in("talk-directional").unwrap();
        for level in [0.06f32, 0.15, 0.5] {
            let room = noise(48000 * 3, 7, level * 0.003);
            let mut v = talk(10.0, level);
            for (a, b) in v.iter_mut().zip(noise(48000 * 10, 9, level * 0.003)) {
                *a += b;
            }
            let r = autoset(&mut chain, &base, &room, &v, TARGET);
            eprintln!("level {level}: floor {:.1} voice {:.1} peak {:.1} gate {} comp {} gr {:.1} makeup {} lufs {:.1} st {:?}", r.floor_db, r.voice_db, r.peak_db, r.preset.exp.thr, r.preset.comp.thr, r.comp_gr_db, r.preset.comp.makeup, r.lufs, r.st);
            assert!((r.lufs - TARGET).abs() < 0.5, "level {level}: {r:?}");
            assert!((r.st[1] - TARGET).abs() < 2.0, "level {level}: {r:?}");
            assert!(r.preset.exp.thr > r.floor_db + 9.0 && r.preset.exp.thr < r.voice_db - 19.0, "level {level}: {r:?}");
            assert!((r.comp_gr_db + 4.0).abs() < 1.5, "level {level}: {r:?}");
            assert!(r.warnings.is_empty(), "level {level}: {:?}", r.warnings);
        }
    }

    #[test]
    fn says_when_nobody_talked() {
        let mut chain = Chain::new(Preset::built_in("talk-directional").unwrap()).unwrap();
        let base = Preset::built_in("talk-directional").unwrap();
        let r = autoset(&mut chain, &base, &noise(48000 * 3, 7, 0.01), &noise(48000 * 10, 9, 0.012), TARGET);
        assert!(r.warnings.contains(&Warning::NoVoice), "{:?}", r.warnings);
    }

    #[test]
    fn says_when_the_mic_is_too_quiet() {
        let mut chain = Chain::new(Preset::built_in("talk-directional").unwrap()).unwrap();
        let base = Preset::built_in("talk-directional").unwrap();
        let r = autoset(&mut chain, &base, &noise(48000 * 3, 7, 1e-5), &talk(10.0, 0.01), TARGET);
        assert!(r.warnings.iter().any(|w| matches!(w, Warning::TooQuiet(_))), "{:?}", r.warnings);
    }

    #[test]
    fn says_when_the_room_is_as_loud_as_the_voice() {
        let mut chain = Chain::new(Preset::built_in("talk-directional").unwrap()).unwrap();
        let base = Preset::built_in("talk-directional").unwrap();
        let room = noise(48000 * 3, 7, 0.05);
        let mut v = talk(10.0, 0.1);
        for (a, b) in v.iter_mut().zip(noise(48000 * 10, 9, 0.05)) {
            *a += b;
        }
        let r = autoset(&mut chain, &base, &room, &v, TARGET);
        assert!(r.warnings.iter().any(|w| matches!(w, Warning::Noisy(_) | Warning::NoVoice)), "{:?}", r.warnings);
    }
}
