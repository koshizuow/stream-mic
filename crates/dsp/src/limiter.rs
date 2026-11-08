//! Lookahead true peak limiter (design/dsp/dynamics.md, measurements.md), after Signalsmith 2022:
//!
//!   e[k]  = peak of sample k and the three 4x-interpolated points after it (true peak)
//!   gmax  = min(1, ceiling / e)            the gain that keeps sample k at the ceiling
//!   m     = min of gmax over lookahead + hold
//!   r     = m, but rising only exponentially (release)
//!   s     = two cascaded box filters, total length = lookahead
//!   y     = x delayed by (lookahead - 1 + interpolation delay) times s
//!
//! Every box output averages values at or below the gain the delayed sample needs, so the output
//! never goes over the ceiling, with no attack filter to overshoot. Matches tools/reflim.py
//! (`limit(..., tp="causal")`).

use crate::Stage;

/// Interpolation filter half length: 16 taps per phase, a windowed sinc. The interpolated points
/// between k and k+1 need x up to k + TP_DELAY.
pub const TP_DELAY: usize = 8;
const TP_TAPS: usize = 2 * TP_DELAY;
const KAISER_BETA: f64 = 8.0;

fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let mut k = 1.0;
    while term > 1e-17 * sum {
        term *= (x / (2.0 * k)).powi(2);
        sum += term;
        k += 1.0;
    }
    sum
}

/// Coefficient for x[k + j] when interpolating at k + p/4, j in -7..=8.
pub fn tp_coef(p: usize, j: i32) -> f64 {
    let tau = p as f64 / 4.0 - j as f64;
    let half = TP_DELAY as f64;
    if tau.abs() >= half {
        return 0.0;
    }
    let sinc = if tau == 0.0 { 1.0 } else { (std::f64::consts::PI * tau).sin() / (std::f64::consts::PI * tau) };
    sinc * bessel_i0(KAISER_BETA * (1.0 - (tau / half).powi(2)).sqrt()) / bessel_i0(KAISER_BETA)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LimParams {
    pub ceil_db: f64,
    pub rel_ms: f64,
    pub look_ms: f64,
    pub hold_ms: f64,
    pub true_peak: bool,
}

impl Default for LimParams {
    fn default() -> Self {
        LimParams { ceil_db: -1.0, rel_ms: 50.0, look_ms: 2.0, hold_ms: 10.0, true_peak: true }
    }
}

/// Fixed-capacity ring buffer, allocated in `prepare` only.
#[derive(Clone, Debug, Default)]
struct Ring {
    buf: Vec<f64>,
    pos: usize,
}

impl Ring {
    fn new(len: usize, fill: f64) -> Ring {
        Ring { buf: vec![fill; len.max(1)], pos: 0 }
    }
    /// Push v, return the value pushed len samples ago.
    #[inline]
    fn push(&mut self, v: f64) -> f64 {
        let old = self.buf[self.pos];
        self.buf[self.pos] = v;
        self.pos += 1;
        if self.pos == self.buf.len() {
            self.pos = 0;
        }
        old
    }
}

#[derive(Clone, Debug)]
struct Box_ {
    ring: Ring,
    sum: f64,
    len: usize,
}

impl Box_ {
    fn new(len: usize) -> Box_ {
        Box_ { ring: Ring::new(len, 1.0), sum: len as f64, len }
    }
    #[inline]
    fn tick(&mut self, v: f64) -> f64 {
        let old = self.ring.push(v);
        self.sum += v - old;
        self.sum / self.len as f64
    }
}

/// Running minimum over the last `w` values (monotonic queue in fixed arrays).
#[derive(Clone, Debug)]
struct MinWindow {
    idx: Vec<u64>,
    val: Vec<f64>,
    head: usize,
    len: usize,
    w: u64,
    n: u64,
}

impl MinWindow {
    fn new(w: usize) -> MinWindow {
        MinWindow { idx: vec![0; w + 1], val: vec![0.0; w + 1], head: 0, len: 0, w: w as u64, n: 0 }
    }
    #[inline]
    fn tick(&mut self, v: f64) -> f64 {
        let cap = self.idx.len();
        while self.len > 0 {
            let back = (self.head + self.len - 1) % cap;
            if self.val[back] >= v {
                self.len -= 1;
            } else {
                break;
            }
        }
        let at = (self.head + self.len) % cap;
        self.idx[at] = self.n;
        self.val[at] = v;
        self.len += 1;
        while self.idx[self.head] + self.w <= self.n {
            self.head = (self.head + 1) % cap;
            self.len -= 1;
        }
        self.n += 1;
        self.val[self.head]
    }
}

#[derive(Clone, Debug)]
pub struct Limiter {
    p: LimParams,
    fs: f64,
    thr: f64,
    k: f64,
    look: usize,
    coefs: [[f64; TP_TAPS]; 3],
    hist: [f64; 2 * TP_TAPS],
    hpos: usize,
    delay: Ring,
    minw: MinWindow,
    rel: f64,
    b1: Box_,
    b2: Box_,
    gain: f64,
}

impl Limiter {
    pub fn new(p: LimParams) -> Self {
        let mut coefs = [[0.0; TP_TAPS]; 3];
        for (pi, row) in coefs.iter_mut().enumerate() {
            for (t, c) in row.iter_mut().enumerate() {
                // t = 0 is x[k - 7], t = 15 is x[k + 8]
                *c = tp_coef(pi + 1, t as i32 - (TP_DELAY as i32 - 1));
            }
        }
        let mut l = Limiter {
            p,
            fs: 48000.0,
            thr: 1.0,
            k: 1.0,
            look: 1,
            coefs,
            hist: [0.0; 2 * TP_TAPS],
            hpos: 0,
            delay: Ring::new(1, 0.0),
            minw: MinWindow::new(1),
            rel: 1.0,
            b1: Box_::new(1),
            b2: Box_::new(1),
            gain: 1.0,
        };
        l.prepare(48000.0, 0);
        l
    }

    /// Ceiling and release change at once (they only scale and slow the gain); lookahead, hold and
    /// true peak change the latency, so they need `prepare`.
    pub fn params(&self) -> &LimParams {
        &self.p
    }

    pub fn set_params(&mut self, p: LimParams) {
        let restart = p.look_ms != self.p.look_ms || p.hold_ms != self.p.hold_ms || p.true_peak != self.p.true_peak;
        self.p = p;
        if restart {
            self.prepare(self.fs, 0);
        } else {
            self.thr = undb(p.ceil_db);
            self.k = 1.0 - (-1.0 / (p.rel_ms.max(0.01) * 1e-3 * self.fs)).exp();
        }
    }

    /// Current gain in dB (<= 0), for the meter.
    pub fn reduction_db(&self) -> f64 {
        20.0 * crate::m::log10(self.gain)
    }

    fn tp_delay(&self) -> usize {
        if self.p.true_peak {
            TP_DELAY
        } else {
            0
        }
    }
}

fn undb(d: f64) -> f64 {
    crate::m::undb(d)
}

impl Stage for Limiter {
    fn prepare(&mut self, fs: f64, _max_block: usize) {
        self.fs = fs;
        self.thr = undb(self.p.ceil_db);
        self.k = 1.0 - (-1.0 / (self.p.rel_ms.max(0.01) * 1e-3 * fs)).exp();
        self.look = ((self.p.look_ms * 1e-3 * fs).round() as usize).max(1);
        let hold = (self.p.hold_ms * 1e-3 * fs).round() as usize;
        let l1 = self.look.div_ceil(2);
        let l2 = self.look - l1 + 1;
        self.hist = [0.0; 2 * TP_TAPS];
        self.hpos = 0;
        // delay by look - 1 + tp: a ring of that length returns the sample pushed that long ago
        let d = self.look - 1 + self.tp_delay();
        self.delay = Ring::new(d.max(1), 0.0);
        self.minw = MinWindow::new(self.look + hold);
        self.b1 = Box_::new(l1);
        self.b2 = Box_::new(l2);
        self.rel = 1.0;
        self.gain = 1.0;
    }

    fn process(&mut self, x: &mut [f32]) {
        let d = self.look - 1 + self.tp_delay();
        for s in x.iter_mut() {
            let v = *s as f64;
            // Each sample is written twice, so the last TP_TAPS are always one contiguous slice.
            self.hist[self.hpos] = v;
            self.hist[self.hpos + TP_TAPS] = v;
            self.hpos = (self.hpos + 1) % TP_TAPS;
            let e = if self.p.true_peak {
                // x[k - 7] ..= x[k + 8], k = n - TP_DELAY; the sample itself and the points at k + 1/4, 2/4, 3/4
                let h = &self.hist[self.hpos..self.hpos + TP_TAPS];
                let mut e = h[TP_DELAY - 1].abs();
                for row in &self.coefs {
                    let acc: f64 = row.iter().zip(h).map(|(c, x)| c * x).sum();
                    e = e.max(acc.abs());
                }
                e
            } else {
                v.abs()
            };
            let gmax = (self.thr / e.max(1e-12)).min(1.0);
            let m = self.minw.tick(gmax);
            self.rel = if m < self.rel { m } else { self.rel + (m - self.rel) * self.k };
            let g = self.b2.tick(self.b1.tick(self.rel));
            self.gain = g;
            let xd = if d == 0 { v } else { self.delay.push(v) };
            *s = (xd * g) as f32;
        }
    }

    fn reset(&mut self) {
        self.prepare(self.fs, 0);
    }

    fn latency(&self) -> usize {
        self.look - 1 + self.tp_delay()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loud_noise(n: usize) -> Vec<f32> {
        let mut s: u32 = 0x9e37_79b9;
        (0..n)
            .map(|i| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                // bursts up to +12 dB over full scale
                let amp = if (i / 4800) % 3 == 1 { 4.0 } else { 0.3 };
                (s as f32 / u32::MAX as f32 - 0.5) * 2.0 * amp
            })
            .collect()
    }

    fn run(p: LimParams, x: &[f32], block: usize) -> Vec<f32> {
        let mut l = Limiter::new(p);
        l.prepare(48000.0, block);
        let mut y = x.to_vec();
        for c in y.chunks_mut(block) {
            l.process(c);
        }
        y
    }

    #[test]
    fn never_over_the_ceiling_and_same_for_any_block() {
        let x = loud_noise(48000);
        for tp in [false, true] {
            let p = LimParams { ceil_db: -1.0, true_peak: tp, ..Default::default() };
            let want = run(p, &x, 1);
            let ceil = 10f32.powf(-1.0 / 20.0);
            let peak = want.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(peak <= ceil * (1.0 + 1e-6), "tp {tp}: peak {peak}");
            for b in [64, 128, 480, 4096] {
                assert_eq!(run(p, &x, b), want, "tp {tp} block {b}");
            }
        }
    }

    #[test]
    fn latency_is_lookahead_plus_interpolation() {
        let l = Limiter::new(LimParams::default());
        assert_eq!(l.latency(), 96 - 1 + TP_DELAY);
    }
}
