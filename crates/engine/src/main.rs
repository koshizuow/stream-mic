//! stream-mic: the real-time engine on its own, without the app window (build-plan.md step 6).
//!
//!   stream-mic [--preset NAME] [--mic NODE] [--out NODE] [--quantum 128] [--port 8790] [--set knob=v,...] [--no-monitor] [--fresh]
//!
//! Makes the virtual mic "stream-mic" and the output "stream-mic 電腦聲音", runs until stopped. The settings page
//! and the control API are at http://127.0.0.1:PORT/. The settings are kept in ~/.config/stream-mic/settings.json;
//! the options given override them, `--fresh` neither reads nor writes it (tests). `--list` prints the audio devices
//! and exits. STREAM_MIC_UI=path serves the page from that file instead of the built-in copy (working on it).

fn main() {
    stream_mic_engine::cli(std::env::args().collect());
}
