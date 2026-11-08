//! stream-mic's effects. Every stage is mono, works on f32 blocks of any size, keeps its state in
//! f64, never allocates or locks in `process`, and gives the same output whatever the block size
//! (parameter changes ramp per sample, not per block).

pub mod deesser;
pub mod dynamics;
pub mod eq;
pub mod limiter;
pub mod m;
pub mod reverb;
pub mod svf;

/// One effect in the chain.
pub trait Stage {
    /// Called before processing and whenever the sample rate changes. May allocate.
    fn prepare(&mut self, fs: f64, max_block: usize);
    /// Process in place. Any length up to `max_block`.
    fn process(&mut self, x: &mut [f32]);
    /// Clear the internal state (filters, envelopes, delay lines) without touching parameters.
    fn reset(&mut self);
    /// Added delay in samples.
    fn latency(&self) -> usize {
        0
    }
}
