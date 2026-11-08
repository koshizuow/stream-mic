//! Following another clock (Windows, build-plan.md step 9): every endpoint there runs on its own clock, and VB-CABLE's
//! ran 0.15 % off the interface's on the Windows test PC (the ring to it dropped 60 to 84 frames a second, for good). An output
//! (or the computer's sound coming in) takes its frames from a ring through this resampler, whose ratio follows how
//! full the ring is: the clocks' difference is taken up smoothly instead of by dropping or padding.
//!
//! - resampling: windowed sinc, 32 taps (Kaiser, beta 8, cut at 0.95 of the lower rate's Nyquist), 128 phases,
//!   linear between two phases; 16 input frames (0.33 ms) of delay. It also changes the rate (a nominal ratio:
//!   44.1 kHz devices on the Windows test PC), since Windows' own conversion (AUTOCONVERTPCM) left images 58 dB under a 1 kHz tone
//! - control: the frames waiting (ring and the resampler's own) less the target, smoothed over 0.5 s (packets of 10 ms
//!   make a sawtooth that must not become a pitch wobble), then proportional and integral; the ratio stays within
//!   1 ± 0.5 %. A clock off by d sends an extra packet every 1/d packets: the ratio follows that step a little

use crate::state::Ring;

const TAPS: usize = 32;
const HALF: usize = TAPS / 2;
const PHASES: usize = 128;
const MAX_DEV: f64 = 0.005;
/// per frame of error, and per frame and second. Tuned in a simulation of two clocks 0.2 % apart in 10 ms packets:
/// the ratio settles on the difference (mean 1.00201) and swings ±0.04 % (under a cent of pitch) as the packets come,
/// at most 270 frames from the target. Larger gains swung twice as much
const KP: f64 = 2e-6;
const KI: f64 = 1e-7;
const SMOOTH_S: f64 = 0.5;

fn bessel_i0(x: f64) -> f64 {
    let (mut s, mut t, mut k) = (1.0, 1.0, 1.0);
    while t > 1e-12 * s {
        t *= (x / (2.0 * k)).powi(2);
        s += t;
        k += 1.0;
    }
    s
}

pub struct Asrc {
    ch: usize,
    /// output frames per second (the control's time base)
    rate: f64,
    /// input frames per output frame when the clocks agree
    nominal: f64,
    /// PHASES + 1 rows of TAPS
    table: Vec<f32>,
    /// input frames taken from the ring, not yet passed: interleaved
    buf: Vec<f32>,
    /// read position in buf, frames (the output's centre is at pos + HALF - 1 + frac)
    pos: f64,
    pub ratio: f64,
    err: f64,
    integ: f64,
    started: bool,
    scratch: Vec<f32>,
}

impl Asrc {
    /// `nominal`: input frames per output frame (input rate / output rate), `rate` the output's
    pub fn new(ch: usize, rate: f64, nominal: f64) -> Asrc {
        let beta = 8.0;
        // in the input's terms: below the output's Nyquist when it is the lower rate
        let cut = 0.95 * (1.0 / nominal).min(1.0);
        let mut table = vec![0.0f32; (PHASES + 1) * TAPS];
        for p in 0..=PHASES {
            let frac = p as f64 / PHASES as f64;
            let row = &mut table[p * TAPS..(p + 1) * TAPS];
            let mut sum = 0.0;
            let mut tmp = [0.0f64; TAPS];
            for (k, t) in tmp.iter_mut().enumerate() {
                // tap k sits at k - (HALF - 1) - frac from the output point
                let x = k as f64 - (HALF as f64 - 1.0) - frac;
                let s = if x.abs() < 1e-12 { cut } else { (std::f64::consts::PI * cut * x).sin() / (std::f64::consts::PI * x) };
                let r = x / HALF as f64;
                let w = if r.abs() >= 1.0 { 0.0 } else { bessel_i0(beta * (1.0 - r * r).sqrt()) / bessel_i0(beta) };
                *t = s * w;
                sum += *t;
            }
            for (o, t) in row.iter_mut().zip(tmp) {
                *o = (t / sum) as f32;
            }
        }
        Asrc { ch, rate, nominal, table, buf: Vec::with_capacity(8192 * ch), pos: 0.0, ratio: nominal, err: 0.0, integ: 0.0, started: false, scratch: Vec::with_capacity(8192 * ch) }
    }

    /// how far the other clock is from what it says, as a ratio (1: the same)
    pub fn drift(&self) -> f64 {
        self.ratio / self.nominal
    }

    /// The rate changed only, no clock to follow (the mic, which the rest follows): all `ring` holds, converted,
    /// appended to `out`
    pub fn convert(&mut self, ring: &mut Ring, out: &mut Vec<f32>) {
        let avail = ring.frames() as f64 + (self.buf.len() / self.ch) as f64 - self.pos - TAPS as f64;
        if avail <= 0.0 {
            return;
        }
        let n = (avail / self.ratio) as usize;
        let start = out.len();
        out.resize(start + n * self.ch, 0.0);
        let mut tmp = std::mem::take(&mut self.scratch);
        self.produce(ring, &mut out[start..], &mut tmp);
        self.scratch = tmp;
    }

    /// `out` from the input at the current ratio; false when the input ran out (the rest silent)
    fn produce(&mut self, ring: &mut Ring, out: &mut [f32], scratch: &mut Vec<f32>) -> bool {
        let ch = self.ch;
        let n = out.len() / ch;
        for i in 0..n {
            let need = self.pos as usize + TAPS;
            let have = self.buf.len() / ch;
            if need > have {
                let want = (need - have).max(256);
                scratch.resize(want * ch, 0.0);
                let got = ring.pop(&mut scratch[..want * ch]) / ch;
                self.buf.extend_from_slice(&scratch[..got * ch]);
                if self.buf.len() / ch < need {
                    out[i * ch..].fill(0.0);
                    return false;
                }
            }
            let base = self.pos as usize;
            let f = (self.pos - base as f64) * PHASES as f64;
            let p = (f as usize).min(PHASES - 1);
            let w = (f - p as f64) as f32;
            let (r0, r1) = (&self.table[p * TAPS..(p + 1) * TAPS], &self.table[(p + 1) * TAPS..(p + 2) * TAPS]);
            for c in 0..ch {
                let mut acc = 0.0f32;
                for k in 0..TAPS {
                    let h = r0[k] + w * (r1[k] - r0[k]);
                    acc += h * self.buf[(base + k) * ch + c];
                }
                out[i * ch + c] = acc;
            }
            self.pos += self.ratio;
        }
        // what has been passed goes
        let gone = self.pos as usize;
        if gone > 0 {
            self.buf.drain(..gone * ch);
            self.pos -= gone as f64;
        }
        // what has been passed goes
        let gone = (self.pos as usize).min(self.buf.len() / ch);
        if gone > 0 {
            self.buf.drain(..gone * ch);
            self.pos -= gone as f64;
        }
        true
    }

    /// frames waiting beyond what the next output needs
    fn waiting(&self, ring: &Ring) -> f64 {
        ring.frames() as f64 + (self.buf.len() / self.ch) as f64 - self.pos - TAPS as f64
    }

    /// Fill `out` (interleaved) from `ring`, keeping about `target` frames waiting. Returns (had enough, frames
    /// dropped): short means silence for the rest and waiting again; far too many (a stall) are dropped.
    pub fn pull(&mut self, ring: &mut Ring, out: &mut [f32], target: usize) -> (bool, usize) {
        let n = out.len() / self.ch;
        if !self.started {
            if ring.frames() < target + TAPS {
                out.fill(0.0);
                return (true, 0);
            }
            self.started = true;
            self.buf.clear();
            self.pos = 0.0;
            self.err = 0.0;
            self.integ = 0.0;
            self.ratio = self.nominal;
            // what came before the reader: down to the target
            ring.trim(target + TAPS);
        }
        let mut dropped = 0;
        if self.waiting(ring) > (target * 8).max(4800) as f64 {
            dropped = ring.trim(target + TAPS);
            self.integ = 0.0;
        }
        let mut tmp = std::mem::take(&mut self.scratch);
        let ok = self.produce(ring, out, &mut tmp);
        self.scratch = tmp;
        if !ok {
            self.started = false;
            return (false, dropped);
        }
        // the clocks: more waiting than the target, read faster
        let e = self.waiting(ring) - target as f64;
        let a = (n as f64 / (SMOOTH_S * self.rate)).min(1.0);
        self.err += a * (e - self.err);
        self.integ += self.err * n as f64 / self.rate;
        self.integ = self.integ.clamp(-MAX_DEV / KI, MAX_DEV / KI);
        self.ratio = self.nominal * (1.0 + KP * self.err + KI * self.integ).clamp(1.0 - MAX_DEV, 1.0 + MAX_DEV);
        (true, dropped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a sine through at a fixed ratio: what is left besides the sine (fitted at the output's frequency)
    #[test]
    fn clean_at_a_fixed_ratio() {
        for (hz, ratio) in [(1000.0, 1.0017), (8000.0, 0.998), (15000.0, 1.004)] {
            let mut a = Asrc::new(1, 48000.0, 1.0);
            let mut ring = Ring::new(48000 * 4, 1);
            let x: Vec<f32> = (0..48000 * 3).map(|i| (0.5 * (2.0 * std::f64::consts::PI * hz * i as f64 / 48000.0).sin()) as f32).collect();
            let mut fed = 0;
            let mut y = vec![0.0f32; 48000];
            // fed as it is used, the ratio held where the test puts it
            for b in y.chunks_mut(480) {
                while ring.frames() < 2000 {
                    ring.push(&x[fed..fed + 480]);
                    fed += 480;
                }
                a.ratio = ratio;
                a.pull(&mut ring, b, 480);
            }
            // the output is the sine at hz / ratio's pace: fit it
            let y = &y[4800..];
            let w = 2.0 * std::f64::consts::PI * hz * ratio / 48000.0;
            let (mut cc, mut ss, mut cs, mut yc, mut ys) = (0.0, 0.0, 0.0, 0.0, 0.0);
            for (i, v) in y.iter().enumerate() {
                let (c, s) = ((w * i as f64).cos(), (w * i as f64).sin());
                cc += c * c;
                ss += s * s;
                cs += c * s;
                yc += *v as f64 * c;
                ys += *v as f64 * s;
            }
            let det = cc * ss - cs * cs;
            let (ac, as_) = ((yc * ss - ys * cs) / det, (ys * cc - yc * cs) / det);
            let res: f64 = y.iter().enumerate().map(|(i, v)| (*v as f64 - ac * (w * i as f64).cos() - as_ * (w * i as f64).sin()).powi(2)).sum::<f64>() / y.len() as f64;
            let amp = ac.hypot(as_);
            let rel = 10.0 * (res / (amp * amp / 2.0)).log10();
            eprintln!("{hz} Hz at {ratio}: amplitude {amp:.4}, the rest {rel:.1} dB");
            assert!((amp - 0.5).abs() < 0.005 && rel < -80.0, "{hz} Hz: {amp} {rel}");
        }
    }

    /// fit a sine of frequency `hz` (at `rate`) to y: (amplitude, what is left in dB under it)
    fn fit(y: &[f32], hz: f64, rate: f64) -> (f64, f64) {
        let w = 2.0 * std::f64::consts::PI * hz / rate;
        let (mut cc, mut ss, mut cs, mut yc, mut ys) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for (i, v) in y.iter().enumerate() {
            let (c, s) = ((w * i as f64).cos(), (w * i as f64).sin());
            cc += c * c;
            ss += s * s;
            cs += c * s;
            yc += *v as f64 * c;
            ys += *v as f64 * s;
        }
        let det = cc * ss - cs * cs;
        let (ac, as_) = ((yc * ss - ys * cs) / det, (ys * cc - yc * cs) / det);
        let res: f64 = y.iter().enumerate().map(|(i, v)| (*v as f64 - ac * (w * i as f64).cos() - as_ * (w * i as f64).sin()).powi(2)).sum::<f64>() / y.len() as f64;
        let amp = ac.hypot(as_);
        (amp, 10.0 * (res / (amp * amp / 2.0)).log10())
    }

    /// 44.1 kHz in to 48 kHz (the mic on the Windows test PC), and 48 kHz out to 44.1 kHz (its headphones): the sine stays, nothing
    /// else comes (Windows' own conversion left images 58 dB under a 1 kHz tone)
    #[test]
    fn changes_the_rate_cleanly() {
        for hz in [1000.0, 8000.0, 15000.0] {
            // up, all at once (convert)
            let x: Vec<f32> = (0..44100 * 2).map(|i| (0.5 * (2.0 * std::f64::consts::PI * hz * i as f64 / 44100.0).sin()) as f32).collect();
            let mut a = Asrc::new(1, 48000.0, 44100.0 / 48000.0);
            let mut ring = Ring::new(48000 * 3, 1);
            let mut y = Vec::new();
            for c in x.chunks(441) {
                ring.push(c);
                a.convert(&mut ring, &mut y);
            }
            let (amp, rest) = fit(&y[4800..y.len() - 4800], hz, 48000.0);
            eprintln!("{hz} Hz 44.1 -> 48 kHz: {} frames for 88200, amplitude {amp:.4}, the rest {rest:.1} dB", y.len());
            assert!((y.len() as f64 - 96000.0).abs() < 64.0 && (amp - 0.5).abs() < 0.005 && rest < -80.0);
            // down, pulled (an output at 44.1 kHz), the ratio held at the nominal one
            let x: Vec<f32> = (0..48000 * 3).map(|i| (0.5 * (2.0 * std::f64::consts::PI * hz * i as f64 / 48000.0).sin()) as f32).collect();
            let mut a = Asrc::new(1, 44100.0, 48000.0 / 44100.0);
            let mut ring = Ring::new(48000 * 4, 1);
            let (mut fed, mut y) = (0, vec![0.0f32; 44100 * 2]);
            for b in y.chunks_mut(441) {
                while ring.frames() < 2000 {
                    ring.push(&x[fed..fed + 480]);
                    fed += 480;
                }
                a.ratio = 48000.0 / 44100.0;
                a.pull(&mut ring, b, 960);
            }
            let (amp, rest) = fit(&y[4410..], hz, 44100.0);
            eprintln!("{hz} Hz 48 -> 44.1 kHz: amplitude {amp:.4}, the rest {rest:.1} dB");
            assert!((amp - 0.5).abs() < 0.005 && rest < -80.0);
        }
    }

    /// two clocks 0.2 % apart, both in 10 ms packets at different moments: the ratio finds the difference and the
    /// ring stays near its target, nothing dropped or padded
    #[test]
    fn follows_another_clock() {
        for drift in [0.002, -0.0015, 0.004] {
            let mut a = Asrc::new(2, 48000.0, 1.0);
            let mut ring = Ring::new(48000, 2);
            // two packets: one is how far the level jumps as they come
            let target = 960;
            let mut made = 0.0f64;
            let mut out = vec![0.0f32; 480 * 2];
            let (mut unders, mut drops, mut worst) = (0, 0, 0.0f64);
            let (mut lo, mut hi, mut sum, mut cnt) = (2.0f64, 0.0f64, 0.0, 0);
            // 120 s in 10 ms steps: the producer's packets come at its own pace
            for step in 0..12000 {
                made += 480.0 * (1.0 + drift);
                while made >= 480.0 {
                    ring.push(&[0.1f32; 960]);
                    made -= 480.0;
                }
                let (ok, d) = a.pull(&mut ring, &mut out, target);
                if step > 100 {
                    unders += !ok as usize;
                    drops += d;
                }
                if step > 6000 {
                    worst = worst.max((a.waiting(&ring) - target as f64).abs());
                    lo = lo.min(a.ratio);
                    hi = hi.max(a.ratio);
                    sum += a.ratio;
                    cnt += 1;
                }
            }
            let mean = sum / cnt as f64;
            eprintln!("drift {drift}: ratio over the last minute {lo:.5}..{hi:.5}, mean {mean:.5}; worst {worst:.0} frames off, unders {unders}, dropped {drops}");
            assert!((mean - (1.0 + drift)).abs() < 1e-4 && hi - lo < 0.0015, "{lo} {hi} {mean}");
            assert!(unders == 0 && drops == 0 && worst < 600.0);
        }
    }
}
