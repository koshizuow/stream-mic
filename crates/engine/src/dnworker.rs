//! Denoise on its own thread (build-plan.md step 6). One 10 ms hop of the model takes 2 to 9 ms, longer than a
//! 128-frame cycle (2.7 ms), so run in the audio callback it made dropouts. Here the audio thread hands the mic to
//! this thread through a lock-free queue and takes the result back from another, margin() samples later than the model
//! alone would give: room for a slow hop.
//!
//! The mic always goes through (cheap copies); the model runs only while the preset has denoise on, otherwise the
//! samples pass straight back, so the timing never changes and turning it on needs no handshake: the model starts
//! from a fresh state.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::Thread;
use std::time::Duration;

use stream_mic_denoise::{Denoise, DenoiseParams};
use stream_mic_dsp::Stage;

use crate::state::Shared;

/// samples of slack for a slow hop. 10 ms was not enough on a Linux test PC (powersave: hops up to 9.5 ms, 30 short in 16 s)
pub fn margin() -> usize {
    std::env::var("STREAM_MIC_DN_MARGIN").ok().and_then(|v| v.parse().ok()).unwrap_or(960)
}

/// One writer, one reader, no locks
pub struct Spsc {
    buf: Box<[UnsafeCell<f32>]>,
    head: AtomicUsize,
    tail: AtomicUsize,
}
unsafe impl Sync for Spsc {}
unsafe impl Send for Spsc {}

impl Spsc {
    pub fn new(cap: usize) -> Spsc {
        Spsc { buf: (0..cap).map(|_| UnsafeCell::new(0.0)).collect(), head: AtomicUsize::new(0), tail: AtomicUsize::new(0) }
    }
    /// writer: as much as fits; returns how much
    pub fn push(&self, x: &[f32]) -> usize {
        let (h, t) = (self.head.load(Ordering::Acquire), self.tail.load(Ordering::Relaxed));
        let n = x.len().min(self.buf.len() - t.wrapping_sub(h));
        for (k, &v) in x[..n].iter().enumerate() {
            unsafe { *self.buf[(t + k) % self.buf.len()].get() = v };
        }
        self.tail.store(t.wrapping_add(n), Ordering::Release);
        n
    }
    /// writer: how much fits now
    pub fn room(&self) -> usize {
        self.buf.len() - self.tail.load(Ordering::Relaxed).wrapping_sub(self.head.load(Ordering::Acquire))
    }
    /// reader: drop up to n; returns how many
    pub fn skip(&self, n: usize) -> usize {
        let (h, t) = (self.head.load(Ordering::Relaxed), self.tail.load(Ordering::Acquire));
        let n = n.min(t.wrapping_sub(h));
        self.head.store(h.wrapping_add(n), Ordering::Release);
        n
    }
    /// reader: how many are waiting
    pub fn waiting(&self) -> usize {
        self.tail.load(Ordering::Acquire).wrapping_sub(self.head.load(Ordering::Relaxed))
    }
    /// reader: up to out.len(); returns how much
    pub fn pop(&self, out: &mut [f32]) -> usize {
        let (h, t) = (self.head.load(Ordering::Relaxed), self.tail.load(Ordering::Acquire));
        let n = out.len().min(t.wrapping_sub(h));
        for (k, o) in out[..n].iter_mut().enumerate() {
            *o = unsafe { *self.buf[(h + k) % self.buf.len()].get() };
        }
        self.head.store(h.wrapping_add(n), Ordering::Release);
        n
    }
}

pub struct DnLink {
    /// set when the link goes (the audio side starting again): the worker ends
    stop: Arc<AtomicBool>,
    pub to: Arc<Spsc>,
    pub from: Arc<Spsc>,
    pub thread: Thread,
    /// samples between handing the mic over and getting it back denoised: the model's latency plus MARGIN
    pub latency: usize,
}

/// Start the worker. The model loads in it (it cannot move between threads), and says its latency when ready.
pub fn start(shared: Arc<Shared>, params: DenoiseParams, rate: f64) -> Result<DnLink, String> {
    let to = Arc::new(Spsc::new(rate as usize));
    let from = Arc::new(Spsc::new(rate as usize));
    // the slack, as silence, before anything comes back
    from.push(&vec![0.0; margin()]);
    let (t2, f2) = (to.clone(), from.clone());
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let (ready, wait) = std::sync::mpsc::channel::<Result<usize, String>>();
    let handle = std::thread::Builder::new()
        .name("denoise".into())
        .spawn(move || {
            let mut d = match Denoise::new(params) {
                Ok(d) => d,
                Err(e) => {
                    let _ = ready.send(Err(e));
                    return;
                }
            };
            d.prepare(rate, 4096);
            // the first hops are slow (the model warming up): run a second of silence before saying ready, then
            // start again from a fresh state
            let mut warm = vec![0.0f32; 480];
            for _ in 0..100 {
                d.process(&mut warm);
            }
            d.reset();
            let _ = ready.send(Ok(d.latency()));
            let mut buf = vec![0.0f32; 4096];
            let mut gen = 0;
            let mut on = false;
            while !stop2.load(Ordering::Acquire) {
                let n = t2.pop(&mut buf);
                if n == 0 {
                    std::thread::park_timeout(Duration::from_millis(2));
                    continue;
                }
                let g = shared.gen.load(Ordering::Acquire);
                if g != gen {
                    if let Ok(c) = shared.cfg.lock() {
                        let now = c.preset.on.denoise;
                        if now && !on {
                            d.reset();
                        }
                        on = now;
                        d.set_params(c.preset.denoise_params());
                    }
                    gen = g;
                }
                if on {
                    let t0 = std::time::Instant::now();
                    d.process(&mut buf[..n]);
                    let us = t0.elapsed().as_micros() as u64;
                    shared.count.dn_worst_us.fetch_max(us, Ordering::Relaxed);
                    if us > 5000 {
                        shared.count.dn_slow.fetch_add(1, Ordering::Relaxed);
                    }
                }
                f2.push(&buf[..n]);
            }
        })
        .map_err(|e| e.to_string())?;
    let model = wait.recv().map_err(|e| e.to_string())??;
    Ok(DnLink { stop, to, from, thread: handle.thread().clone(), latency: model + margin() })
}

impl Drop for DnLink {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.thread.unpark();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spsc_in_order_across_threads() {
        let q = Arc::new(Spsc::new(64));
        let w = q.clone();
        let h = std::thread::spawn(move || {
            let mut i = 0u32;
            while i < 10000 {
                let x: Vec<f32> = (i..(i + 7).min(10000)).map(|v| v as f32).collect();
                let n = w.push(&x);
                i += n as u32;
            }
        });
        let mut got = Vec::new();
        let mut b = [0.0; 5];
        while got.len() < 10000 {
            let n = q.pop(&mut b);
            got.extend_from_slice(&b[..n]);
        }
        h.join().unwrap();
        assert!(got.iter().enumerate().all(|(i, &v)| v == i as f32));
    }
}
