//! Loop detection (routing.md rule 2): OBS or Discord monitoring "stream-mic" into "stream-mic 電腦聲音" sends the
//! voice back in as computer sound, and the loopback would send it out again, louder each time round.
//!
//! The audio thread hands over, sample by sample, the voice (before any computer sound is added), the computer sound
//! it takes in, and what it sends out (voice plus loopback). Twice a second this thread looks at the last 3 s, two ways:
//!
//! - exact: is our voice in the computer sound? What we send minus the voice is the loopback, so in a loop the
//!   computer sound is, some time d later: the voice, plus the loopback going round again, plus any other music.
//!   d is found by correlating the computer sound with what we sent (at 8 kHz, 0..500 ms), then sample by sample: the
//!   computer sound is fitted on the voice and the loopback, both d earlier, and what the voice adds beyond the
//!   loopback (the partial correlation) must be at least 0.3, twice in a row. Without a loop the voice adds nothing
//!   (about 0.01), whatever the music. Matching what we sent as a whole failed both ways: once the sound has gone
//!   round, it matches itself at many delays, and a held synth note whose period is a whole number of samples
//!   matches the loopback's copy of it (seen on a Linux test PC, and in the tests below).
//! - envelope: the computer sound's loudness follows the voice's syllable by syllable (`detail`: correlation over
//!   0..500 ms at least 0.8, both loud enough to judge). For a path that changes the samples (a filter in OBS,
//!   Discord's processing). It loses a loop that has gone round a few times (on a Linux test PC the computer sound grew to
//!   1.8 times full scale in a second).
//!
//! Both are fooled when the mic itself hears the computer sound (speakers instead of headphones): the voice is then
//! the computer sound a little later, and a held note whose period is a whole number of samples, or any music's
//! smooth loudness, matches it the other way round too (seen on a Linux test PC with the mic on the headphones' monitor).
//! So first: how much is the voice the computer sound of up to 100 ms before ("bleed", sample by sample)? While it is
//! 0.4 or more the envelope way is not used, and the exact way must beat it by 0.2.
//!
//! Either one mutes the loopback and the page says why. Ten seconds without either and it comes back.

use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::state::Shared;

/// envelope points per second
pub const ENV_RATE: usize = 1000;
const WINDOW_S: usize = 3;
const MAX_LAG: usize = ENV_RATE / 2;
const THRESHOLD: f64 = 0.8;
const PARTIAL: f64 = 0.3;
/// the exact way's first pass runs at rate / DECIM
const DECIM: usize = 6;
const HOLD: Duration = Duration::from_secs(10);
/// detail(): the average taken out spans 2 * DETAIL + 1 envelope points (about 200 ms, a syllable or two)
const DETAIL: usize = ENV_RATE / 10;
/// the envelope way needs syllables to compare: rms(detail) / mean(envelope) at least this, the voice and the
/// computer sound. Speech at least 0.50 (16 DAPS recordings and the preview's voice, 3 s windows), held music 0.06, a
/// tone 0: two held sounds that start together left only that start to compare, and matched (tools/func_test.py)
const VOICE_MOD: f64 = 0.3;
const PC_MOD: f64 = 0.2;

/// normalized correlation of `p` against `v` delayed by each of 0..=max_lag points
pub fn follow_all(v: &[f32], p: &[f32], max_lag: usize) -> Vec<f64> {
    let n = v.len().min(p.len());
    (0..=max_lag.min(n.saturating_sub(100)))
        .map(|lag| {
            let (a, b) = (&v[..n - lag], &p[lag..n]);
            let m = a.len() as f64;
            let (ma, mb) = (a.iter().map(|&x| x as f64).sum::<f64>() / m, b.iter().map(|&x| x as f64).sum::<f64>() / m);
            let (mut sab, mut saa, mut sbb) = (0.0, 0.0, 0.0);
            for (&x, &y) in a.iter().zip(b) {
                let (x, y) = (x as f64 - ma, y as f64 - mb);
                sab += x * y;
                saa += x * x;
                sbb += y * y;
            }
            if saa > 0.0 && sbb > 0.0 {
                sab / (saa * sbb).sqrt()
            } else {
                0.0
            }
        })
        .collect()
}

/// The best of follow_all and its delay
pub fn follow(v: &[f32], p: &[f32], max_lag: usize) -> (f64, usize) {
    follow_all(v, p, max_lag).into_iter().enumerate().fold((0.0, 0), |b, (lag, r)| if r > b.0 { (r, lag) } else { b })
}

/// mean absolute value per `per` samples
pub fn envelope(x: &[f32], per: usize) -> Vec<f32> {
    x.chunks_exact(per).map(|c| c.iter().map(|v| v.abs()).sum::<f32>() / per as f32).collect()
}

/// How much an envelope moves syllable by syllable: rms(detail) / mean, the less of its two halves (a sound that
/// starts late in the window is all start in its half, and nothing in the other)
pub fn modulation(e: &[f32], d: &[f32]) -> f64 {
    let h = e.len().min(d.len()) / 2;
    let one = |e: &[f32], d: &[f32]| {
        let m = e.iter().map(|&x| x as f64).sum::<f64>() / e.len().max(1) as f64;
        let r = (d.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / d.len().max(1) as f64).sqrt();
        if m > 0.0 { r / m } else { 0.0 }
    };
    one(&e[..h], &d[..h]).min(one(&e[h..2 * h], &d[h..2 * h]))
}

/// The envelope way: the computer sound's syllables following the voice's, (correlation, delay in points); 0 when
/// either has too little of them to judge
pub fn envelope_follow(se: &[f32], pe: &[f32]) -> (f64, usize) {
    let (sd, pd) = (detail(se, DETAIL), detail(pe, DETAIL));
    if modulation(se, &sd) < VOICE_MOD || modulation(pe, &pd) < PC_MOD {
        return (0.0, 0);
    }
    follow(&sd, &pd, MAX_LAG)
}

/// An envelope less its own average over `half` points either side: the syllables, not the phrases. Two unrelated
/// sounds that start and stop together match on the whole envelope whatever they are (on a Linux test PC the preview's voice
/// and music loops did every few minutes, and the loopback was muted for nothing); only a copy matches syllable by
/// syllable.
pub fn detail(e: &[f32], half: usize) -> Vec<f32> {
    let mut sum = Vec::with_capacity(e.len() + 1);
    sum.push(0.0f64);
    for &x in e {
        sum.push(sum.last().unwrap() + x as f64);
    }
    (0..e.len())
        .map(|i| {
            let (a, b) = (i.saturating_sub(half), (i + half + 1).min(e.len()));
            e[i] - ((sum[b] - sum[a]) / (b - a) as f64) as f32
        })
        .collect()
}

/// Is our voice `s` in the computer sound `p`, beyond the loopback? `o` is what we sent (the voice plus the loopback).
/// All at `rate`, the same length, at least 1 s. The partial correlation of the last 0.5 s of `p` with `s` d samples
/// earlier, the loopback (o - s) d earlier held out, at the best d up to MAX_LAG ms; and that d.
pub fn voice_in(s: &[f32], o: &[f32], p: &[f32], rate: usize) -> (f64, usize) {
    let n = s.len().min(o.len()).min(p.len());
    let w = rate / 2;
    let max_d = rate * MAX_LAG / ENV_RATE;
    if n < w + max_d + 2 * DECIM {
        return (0.0, 0);
    }
    // first pass: plain correlation of p with what we sent, at rate / DECIM (box averages), the peaks. In a
    // loop p is exactly what we sent, d later (with the voice alone the delay is lost once the sound has gone round:
    // the sum of every round swamps it); the copies a round later match almost as well, hence several
    let dec = |x: &[f32]| -> Vec<f32> { x.chunks_exact(DECIM).map(|c| c.iter().sum::<f32>() / DECIM as f32).collect() };
    let (sd, pd) = (dec(&o[n % DECIM..n]), dec(&p[n % DECIM..n]));
    let (m, wd, md) = (sd.len(), w / DECIM, max_d / DECIM);
    let pw = &pd[m - wd..];
    let ep: f32 = pw.iter().map(|x| x * x).sum();
    if ep <= 0.0 {
        return (0.0, 0);
    }
    let c: Vec<f32> = (0..=md)
        .map(|d| {
            let a = &sd[m - wd - d..m - d];
            let (ab, aa) = a.iter().zip(pw).fold((0.0f32, 0.0f32), |(ab, aa), (x, y)| (ab + x * y, aa + x * x));
            if aa > 0.0 { ab.abs() / (aa * ep).sqrt() } else { 0.0 }
        })
        .collect();
    let peaks: Vec<usize> = (0..c.len()).filter(|&i| (i == 0 || c[i] >= c[i - 1]) && (i + 1 == c.len() || c[i] >= c[i + 1])).collect();
    // the first four strong ones (the loop's own delay is the earliest of its rounds) and the best four
    let top = peaks.iter().map(|&i| c[i]).fold(0.0f32, f32::max);
    let mut cand: Vec<usize> = peaks.iter().copied().filter(|&i| c[i] >= 0.8 * top).take(4).collect();
    let mut by_value = peaks.clone();
    by_value.sort_by(|a, b| c[*b].total_cmp(&c[*a]));
    cand.extend(by_value.into_iter().take(4));
    cand.sort_unstable();
    cand.dedup();
    // second pass: sample by sample around each, the partial correlation
    let pw = &p[n - w..n];
    let mut best = (0.0, 0);
    for &k in &cand {
        let centre = k * DECIM;
        for d in centre.saturating_sub(DECIM)..=(centre + DECIM).min(max_d) {
            let (a, ob) = (&s[n - w - d..n - d], &o[n - w - d..n - d]);
            let (mut pp, mut aa, mut bb, mut pa, mut pb, mut ab) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
            for i in 0..w {
                let (y, x1) = (pw[i] as f64, a[i] as f64);
                let x2 = ob[i] as f64 - x1;
                pp += y * y;
                aa += x1 * x1;
                bb += x2 * x2;
                pa += y * x1;
                pb += y * x2;
                ab += x1 * x2;
            }
            if pp <= 0.0 || aa <= 0.0 {
                continue;
            }
            let r = if bb > 0.0 {
                let (rpa, rpb, rab) = (pa / (pp * aa).sqrt(), pb / (pp * bb).sqrt(), ab / (aa * bb).sqrt());
                let den = ((1.0 - rpb * rpb).max(1e-12) * (1.0 - rab * rab).max(1e-12)).sqrt();
                (rpa - rpb * rab) / den
            } else {
                pa / (pp * aa).sqrt()
            };
            if r > best.0 {
                best = (r, d);
            }
        }
    }
    best
}

/// How much the voice `s` is the computer sound `p` of up to 100 ms before (the mic hearing speakers): the best
/// normalized correlation of all of `s` (but the first 100 ms) with `p` that much earlier, found at rate / DECIM and
/// refined. The whole window, as the envelope way looks at: over less, a pause in the music read as no bleed while
/// the envelopes still matched from before (a Linux test PC)
pub fn bleed(s: &[f32], p: &[f32], rate: usize) -> f64 {
    let n = s.len().min(p.len());
    let max_x = rate / 10;
    let w = n.saturating_sub(max_x + 2 * DECIM);
    if w < rate / 2 {
        return 0.0;
    }
    let ncc = |a: &[f32], b: &[f32]| {
        let (mut ab, mut aa, mut bb) = (0.0f64, 0.0f64, 0.0f64);
        for (&x, &y) in a.iter().zip(b) {
            ab += x as f64 * y as f64;
            aa += (x as f64).powi(2);
            bb += (y as f64).powi(2);
        }
        if aa > 0.0 && bb > 0.0 { ab.abs() / (aa * bb).sqrt() } else { 0.0 }
    };
    let dec = |x: &[f32]| -> Vec<f32> { x.chunks_exact(DECIM).map(|c| c.iter().sum::<f32>() / DECIM as f32).collect() };
    let (sd, pd) = (dec(&s[n % DECIM..n]), dec(&p[n % DECIM..n]));
    let (m, wd) = (sd.len(), w / DECIM);
    let sw = &sd[m - wd..];
    let best = (0..=max_x / DECIM).map(|x| (ncc(sw, &pd[m - wd - x..m - x]), x)).fold((0.0, 0), |b, c| if c.0 > b.0 { c } else { b });
    let sw = &s[n - w..n];
    ((best.1 * DECIM).saturating_sub(DECIM)..=(best.1 * DECIM + DECIM).min(max_x))
        .map(|x| ncc(sw, &p[n - w - x..n - x]))
        .fold(best.0, f64::max)
}

pub fn start(shared: Arc<Shared>, rate: usize) {
    let _ = std::thread::Builder::new().name("loop-detect".into()).spawn(move || {
        let keep = WINDOW_S * rate;
        let (mut sv, mut pv, mut ov) = (VecDeque::with_capacity(keep), VecDeque::with_capacity(keep), VecDeque::with_capacity(keep));
        let mut buf = vec![0.0f32; 3 * rate];
        let mut last_hit: Option<Instant> = None;
        let mut exact_in_row = 0;
        let mut tick = 0u32;
        let debug = std::env::var_os("LOOP_DEBUG").is_some();
        loop {
            std::thread::sleep(Duration::from_millis(100));
            // the tap holds a second: drain it often
            let n = shared.tap.pop(&mut buf) / 3 * 3;
            for t in buf[..n].chunks_exact(3) {
                for (q, x) in [(&mut sv, t[0]), (&mut pv, t[1]), (&mut ov, t[2])] {
                    if q.len() == keep {
                        q.pop_front();
                    }
                    q.push_back(x);
                }
            }
            tick += 1;
            if tick % 5 != 0 || sv.len() < keep {
                continue;
            }
            let (s, p, o) = (sv.make_contiguous().to_vec(), pv.make_contiguous().to_vec(), ov.make_contiguous().to_vec());
            let per = rate / ENV_RATE;
            let (se, pe, oe) = (envelope(&s, per), envelope(&p, per), envelope(&o, per));
            let loud = |x: &[f32]| x.iter().map(|&a| a as f64).sum::<f64>() / x.len() as f64;
            // mean absolute level: the voice at least about -50 dBFS, the computer sound about -60
            let pc_on = loud(&pe) > 1e-3;
            let (re, le) = if pc_on && loud(&se) > 3e-3 { envelope_follow(&se, &pe) } else { (0.0, 0) };
            let (rx, dx) = if pc_on && loud(&se) > 3e-3 { voice_in(&s, &o, &p, rate) } else { (0.0, 0) };
            let bl = if pc_on && loud(&se) > 3e-3 { bleed(&s, &p, rate) } else { 0.0 };
            exact_in_row = if rx >= PARTIAL && rx > bl + 0.2 { exact_in_row + 1 } else { 0 };
            let re = if bl < 0.4 { re } else { 0.0 };
            if debug {
                eprintln!("loop: voice {:.4} pc {:.4} sent {:.4} envelope {re:.3} at {le} ms, voice in it {rx:.4} at {dx}, mic hears pc {bl:.3}", loud(&se), loud(&pe), loud(&oe));
            }
            let (r, lag_ms) = if rx >= re { (rx, dx * 1000 / rate) } else { (re, le) };
            shared.loop_corr.store((r * 1000.0) as u64, Ordering::Relaxed);
            shared.loop_lag_ms.store(lag_ms as u64, Ordering::Relaxed);
            if re >= THRESHOLD || exact_in_row >= 2 {
                if !shared.loop_mute.swap(true, Ordering::AcqRel) {
                    eprintln!("feedback: the computer sound is our own output coming back; loopback muted (is OBS or Discord monitoring stream-mic into stream-mic 電腦聲音?)");
                }
                last_hit = Some(Instant::now());
            } else if last_hit.is_some_and(|t| t.elapsed() > HOLD) {
                shared.loop_mute.store(false, Ordering::Release);
                last_hit = None;
                eprintln!("feedback gone: loopback back on");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a syllable-like signal: noise under an on-off envelope, lowpassed a little
    fn voice(n: usize, seed: u64, on: usize) -> Vec<f32> {
        let mut x = seed;
        let mut y = 0.0f32;
        (0..n)
            .map(|i| {
                x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let r = ((x >> 33) as f32 / (1u64 << 31) as f32) - 0.5;
                y = 0.7 * y + 0.3 * r;
                y * if (i / on) % 3 == 0 { 1.2 } else { 0.1 }
            })
            .collect()
    }

    /// a loop: what we send comes back `d` samples later with any `music`, and goes out again `lb` samples later
    /// (clipped, as the sink would get it)
    fn looped(s: &[f32], music: &[f32], d: usize, lb: usize, clip: f32) -> (Vec<f32>, Vec<f32>) {
        let n = s.len();
        let (mut o, mut p) = (vec![0.0f32; n], vec![0.0f32; n]);
        for i in 0..n {
            p[i] = (if i >= d { o[i - d] } else { 0.0 } + music[i]).clamp(-clip, clip);
            o[i] = s[i] + if i >= lb { p[i - lb] } else { 0.0 };
        }
        (o, p)
    }

    /// no loop: the music comes in, we send the voice plus the music `lb` samples later
    fn normal(s: &[f32], music: &[f32], lb: usize) -> Vec<f32> {
        (0..s.len()).map(|i| s[i] + if i >= lb { music[i - lb] } else { 0.0 }).collect()
    }

    #[test]
    fn finds_a_delayed_copy_not_something_else() {
        // a syllable-like envelope, and the same 120 points later at half the level
        let v: Vec<f32> = (0..3000).map(|i| (((i * 7919) % 401) as f32 / 400.0) * if (i / 150) % 3 == 0 { 1.0 } else { 0.1 }).collect();
        let mut p = vec![0.0f32; 3000];
        for i in 120..3000 {
            p[i] = 0.5 * v[i - 120];
        }
        let (r, lag) = follow(&v, &p, 500);
        assert!(r > 0.95 && lag == 120, "{r} {lag}");
        // unrelated: music that has nothing to do with the voice
        let q: Vec<f32> = (0..3000).map(|i| (((i * 104729) % 977) as f32 / 977.0) * if (i / 400) % 2 == 0 { 1.0 } else { 0.3 }).collect();
        let (r, _) = follow(&v, &q, 500);
        assert!(r < 0.5, "{r}");
    }

    #[test]
    fn hears_the_voice_in_a_loop_that_has_gone_round() {
        let rate = 48000;
        let n = 3 * rate;
        let s = voice(n, 1, 9000);
        let silence = vec![0.0f32; n];
        // music twice as loud as the voice, going round too
        let music: Vec<f32> = voice(n, 2, 7000).iter().map(|x| 2.0 * x).collect();
        for (what, m, clip) in [("no music", &silence, 1.5), ("no music, not clipped", &silence, 1e9), ("music", &music, 1.5), ("music, not clipped", &music, 1e9)] {
            for (d, lb) in [(128, 103), (1000, 200), (20000, 103)] {
                let (o, p) = looped(&s, m, d, lb, clip);
                let (r, got) = voice_in(&s, &o, &p, rate);
                assert!(r >= PARTIAL && got == d, "{what}, d {d}: {r:.3} at {got}");
                // the envelope way does not see it any more
                let (re, _) = follow(&envelope(&s, 48), &envelope(&p, 48), 500);
                eprintln!("{what}, d {d}: voice in it {r:.3}, envelope {re:.3}");
            }
        }
    }

    #[test]
    fn speakers_into_the_mic_are_not_a_loop() {
        // the mic hears the computer sound (speakers, 300 samples later, half as loud) and a little voice; the music
        // is a held square note of period 100 samples, played in a rhythm
        let rate = 48000;
        let n = 3 * rate;
        let v = voice(n, 5, 9000);
        let note: Vec<f32> = (0..n).map(|i| if (i / 6000) % 2 == 0 { 0.0 } else if i % 100 < 50 { 0.3 } else { -0.3 }).collect();
        let noise: Vec<f32> = voice(n, 6, 5000);
        for (what, m) in [("note", &note), ("noise", &noise)] {
            let s: Vec<f32> = (0..n).map(|i| 0.1 * v[i] + if i >= 300 { 0.5 * m[i - 300] } else { 0.0 }).collect();
            let o = normal(&s, m, 200);
            let (r, _) = voice_in(&s, &o, m, rate);
            let bl = bleed(&s, m, rate);
            assert!(bl >= 0.4 && !(r >= PARTIAL && r > bl + 0.2), "{what}: voice in it {r:.3}, bleed {bl:.3}");
        }
        // and a real loop is still one: the voice does not much follow the computer sound there
        let s = voice(n, 7, 9000);
        let (o, p) = looped(&s, &noise, 1000, 103, 1.5);
        let (r, _) = voice_in(&s, &o, &p, rate);
        let bl = bleed(&s, &p, rate);
        assert!(r >= PARTIAL && r > bl + 0.2, "loop: {r:.3}, bleed {bl:.3}");
    }

    /// speech-like: syllables of 120..280 ms at random levels with short gaps, in phrases `on` samples long every
    /// `period` samples starting at `start`, over lowpassed noise
    fn speech(n: usize, seed: u64, start: usize, on: usize, period: usize) -> Vec<f32> {
        let mut x = seed;
        let mut rnd = move || {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (x >> 33) as f32 / (1u64 << 31) as f32
        };
        let mut level = vec![0.0f32; n];
        let mut i = 0;
        while i < n {
            let len = 5760 + (rnd() * 7680.0) as usize;
            let a = 0.2 + 0.8 * rnd();
            for v in level.iter_mut().skip(i).take(len) {
                *v = a;
            }
            i += len + 1440 + (rnd() * 1440.0) as usize;
        }
        let mut y = 0.0f32;
        (0..n)
            .map(|i| {
                y = 0.7 * y + 0.3 * (rnd() - 0.5);
                let in_phrase = i >= start && (i - start) % period < on;
                if in_phrase { y * level[i] } else { 0.0 }
            })
            .collect()
    }

    fn envelope_way(s: &[f32], p: &[f32]) -> (f64, f64) {
        let (se, pe) = (envelope(s, 48), envelope(p, 48));
        (follow(&se, &pe, MAX_LAG).0, envelope_follow(&se, &pe).0)
    }

    #[test]
    fn held_sounds_that_start_together_are_not_a_loop() {
        // two tones starting in the same window: nothing but that start to compare (it matched, 0.8 and more)
        let rate = 48000;
        let n = 3 * rate;
        let tone = |hz: f32, amp: f32, start: usize| -> Vec<f32> {
            (0..n).map(|i| if i < start { 0.0 } else { amp * (2.0 * std::f32::consts::PI * hz * i as f32 / rate as f32).sin() }).collect()
        };
        for start in [10000, 60000, 70000, 120000, 140000] {
            let (whole, syl) = envelope_way(&tone(440.0, 0.1, start), &tone(1000.0, 0.05, start + 500));
            eprintln!("start {start}: whole envelope {whole:.3}, syllables {syl:.3}");
            assert!(syl < 0.5, "start {start}: {syl:.3}");
        }
    }

    #[test]
    fn sounds_that_start_together_are_not_a_loop() {
        // the voice and unrelated music in phrases that begin and end together (music 100 ms after the voice)
        let rate = 48000;
        let n = 3 * rate;
        // music held at one level: its envelope is all phrase, and the whole-envelope way took it for a loop
        let mut worst_whole = 0.0f64;
        for seed in 0..6 {
            let s = speech(n, 10 + seed, 0, 60000, 90000);
            let held: Vec<f32> = voice(n, 20 + seed, n).iter().enumerate().map(|(i, x)| if i >= 4800 && (i - 4800) % 90000 < 60000 { *x } else { 0.0 }).collect();
            // as seen on a Linux test PC: both had been going and stop together, the music 90 ms after the voice
            let talk = speech(n, 50 + seed, 0, 115200, 2 * n);
            let ends: Vec<f32> = voice(n, 60 + seed, n).iter().enumerate().map(|(i, x)| if i < 115200 + 4320 { 0.5 * x } else { 0.0 }).collect();
            for (what, v, m) in [
                ("speech-like music", &s, speech(n, 20 + seed, 4800, 60000, 90000)),
                ("held music", &s, held),
                ("both stop", &talk, ends),
            ] {
                let (whole, syl) = envelope_way(v, &m);
                eprintln!("seed {seed} {what}: whole envelope {whole:.3}, syllables {syl:.3}");
                assert!(syl < 0.5, "seed {seed} {what}: {syl:.3}");
                worst_whole = worst_whole.max(whole);
            }
        }
        // these stay under the whole envelope's 0.8 too (0.37 to 0.60): the real case (the preview's loops, 0.83) is
        // checked live on a Linux test PC
        eprintln!("whole envelope at most {worst_whole:.3}");
    }

    #[test]
    fn a_changed_copy_is_still_a_loop() {
        // what we send comes back 200 ms later, filtered, quieter, under music of its own
        let rate = 48000;
        let n = 3 * rate;
        for seed in 0..6 {
            let s = speech(n, 30 + seed, 0, 60000, 90000);
            let m = speech(n, 40 + seed, 20000, 50000, 70000);
            for (what, music) in [("alone", 0.0f32), ("under quiet music", 0.1)] {
                let mut y = 0.0f32;
                let p: Vec<f32> = (0..n)
                    .map(|i| {
                        y = 0.8 * y + 0.2 * if i >= 9600 { s[i - 9600] } else { 0.0 };
                        0.5 * y + music * m[i]
                    })
                    .collect();
                let (whole, syl) = envelope_way(&s, &p);
                eprintln!("seed {seed} {what}: whole envelope {whole:.3}, syllables {syl:.3}");
                assert!(syl >= THRESHOLD, "seed {seed} {what}: {syl:.3}");
            }
        }
    }

    #[test]
    fn no_loop_whatever_the_music() {
        let rate = 48000;
        let n = 3 * rate;
        let s = voice(n, 3, 9000);
        // noise-like music, and a held square note of period 100 samples played in a rhythm: the loopback's copy 200
        // samples later lines up with it exactly
        let noise: Vec<f32> = voice(n, 4, 5000).iter().map(|x| 2.0 * x).collect();
        let note: Vec<f32> = (0..n).map(|i| if (i / 6000) % 2 == 0 { 0.0 } else if i % 100 < 50 { 0.3 } else { -0.3 }).collect();
        for (what, m) in [("noise", &noise), ("note", &note)] {
            for quiet in [1.0, 0.1] {
                let sq: Vec<f32> = s.iter().map(|v| v * quiet).collect();
                let o = normal(&sq, m, 200);
                let (r, d) = voice_in(&sq, &o, m, rate);
                assert!(r < 0.1, "{what}, voice x{quiet}: {r:.3} at {d}");
            }
        }
    }
}
