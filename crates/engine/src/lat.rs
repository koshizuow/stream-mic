//! Latency test (build-plan.md step 6): `stream-mic --latency PLAY_NODE REC_NODE [SECONDS] [QUANTUM]`.
//! REC_NODE as `src:NAME` listens to a source, e.g. an interface's input with its output cabled back to it.
//!
//! One client, two streams on the same audio thread: an output into PLAY_NODE (the test mic, a sink whose monitor
//! the engine takes as its mic) sending a click every half second, and an input from REC_NODE ("stream-mic").
//! Each callback reads the driver's clock position for its cycle (the same for every node driven in it), so a
//! frame's time is that plus its index, and the latency is when the click came back minus when it went out.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use pipewire as pw;
use pw::spa;
use pw::spa::pod::Pod;

use crate::audio::{format, input_f32, props};
use crate::proc::RATE;

/// When this cycle started, in frames of monotonic time (the driver's clock.nsec, spa_io_position handed over in
/// io_changed). The click goes into the test mic, which drives its own graph, and comes back from "stream-mic",
/// driven by another: their positions count on different clocks, their start times on the same one. 0 until known.
fn pos(p: *const pw::spa::sys::spa_io_position) -> u64 {
    if p.is_null() {
        0
    } else {
        let ns = unsafe { (*p).clock.nsec };
        ns * RATE as u64 / 1_000_000_000
    }
}

struct St {
    count: u64,
    out_n: u64,
    in_n: u64,
    sent: Vec<u64>,
    got: Vec<u64>,
    last_hit: u64,
    scratch: Vec<f32>,
}

pub fn run(play: &str, rec: &str, secs: f64, quantum: u32) -> Result<(), String> {
    // "src:NAME" listens to a source (a real interface's input, for a cable loop) instead of a sink's monitor
    let (rec, is_source) = match rec.strip_prefix("src:") { Some(r) => (r, true), None => (rec, rec == crate::audio::SOURCE_NAME) };
    pw::init();
    let mainloop = pw::main_loop::MainLoop::new(None).map_err(|e| e.to_string())?;
    let context = pw::context::Context::new(&mainloop).map_err(|e| e.to_string())?;
    let core = context.connect(None).map_err(|e| e.to_string())?;
    let st = Rc::new(RefCell::new(St { count: 0, out_n: 0, in_n: 0, sent: vec![], got: vec![], last_hit: 0, scratch: Vec::with_capacity(16384) }));
    let latency = format!("{quantum}/{RATE}");
    let flags = pw::stream::StreamFlags::MAP_BUFFERS | pw::stream::StreamFlags::RT_PROCESS | pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::DONT_RECONNECT;
    let period = RATE as u64 / 2;

    let out = pw::stream::Stream::new(&core, "latency click", props(&[("media.type", "Audio"), ("media.category", "Playback"), ("node.latency", &latency), ("target.object", play), ("node.dont-fallback", "true")]))
        .map_err(|e| e.to_string())?;
    let s = st.clone();
    let _lo = out
        .add_local_listener_with_user_data(std::ptr::null::<pw::spa::sys::spa_io_position>())
        .io_changed(|_, p, id, area, _| {
            if id == pw::spa::sys::SPA_IO_Position {
                *p = area as *const _;
            }
        })
        .process(move |stream, p| {
            let Some(mut b) = stream.dequeue_buffer() else { return };
            let req = b.requested() as usize;
            let datas = b.datas_mut();
            let Some(d) = datas.first_mut() else { return };
            let mut st = s.borrow_mut();
            let mut frames = 0;
            st.out_n = pos(*p);
            // no clock yet: silence (a click at position 0 would be measured against nothing). One click per half
            // second of this stream's own frames (the start times jump by a cycle each time)
            let live = st.out_n != 0;
            if let Some(bytes) = d.data() {
                frames = if req == 0 { (bytes.len() / 4).min(1024) } else { req.min(bytes.len() / 4) };
                for i in 0..frames {
                    let k = st.count + i as u64;
                    let v: f32 = if live && k % period < 2 { 0.5 } else { 0.0 };
                    if live && k % period == 0 {
                        let at = st.out_n + i as u64;
                        st.sent.push(at);
                    }
                    bytes[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
                }
                if live {
                    st.count += frames as u64;
                }
            }
            let c = d.chunk_mut();
            *c.offset_mut() = 0;
            *c.stride_mut() = 4;
            *c.size_mut() = (frames * 4) as u32;
        })
        .register()
        .map_err(|e| e.to_string())?;
    let p = format(1);
    out.connect(spa::utils::Direction::Output, None, flags, &mut [Pod::from_bytes(&p).unwrap()]).map_err(|e| e.to_string())?;

    let inp = pw::stream::Stream::new(&core, "latency listen", props(&[("media.type", "Audio"), ("media.category", "Capture"), ("node.latency", &latency), ("target.object", rec), ("node.dont-fallback", "true"),
            // a sink (the headphones stand-in): listen to its monitor
            ("stream.capture.sink", if is_source { "false" } else { "true" })]))
        .map_err(|e| e.to_string())?;
    let s = st.clone();
    let _li = inp
        .add_local_listener_with_user_data(std::ptr::null::<pw::spa::sys::spa_io_position>())
        .io_changed(|_, p, id, area, _| {
            if id == pw::spa::sys::SPA_IO_Position {
                *p = area as *const _;
            }
        })
        .process(move |stream, p| {
            let Some(mut b) = stream.dequeue_buffer() else { return };
            let mut st = s.borrow_mut();
            st.in_n = pos(*p);
            let mut x = std::mem::take(&mut st.scratch);
            input_f32(&mut b, &mut x);
            for (i, v) in x.iter().enumerate() {
                let n = st.in_n + i as u64;
                // the click comes back through the chain (gain, limiting): the first sample over a threshold, then
                // deaf for a quarter second
                if v.abs() > 0.05 && n > st.last_hit + period / 2 {
                    st.got.push(n);
                    st.last_hit = n;
                }
            }
            st.scratch = x;
        })
        .register()
        .map_err(|e| e.to_string())?;
    let p = format(1);
    inp.connect(spa::utils::Direction::Input, None, flags, &mut [Pod::from_bytes(&p).unwrap()]).map_err(|e| e.to_string())?;

    let ml = mainloop.clone();
    let timer = mainloop.loop_().add_timer(move |_| ml.quit());
    timer.update_timer(Some(Duration::from_secs_f64(secs)), None).into_result().map_err(|e| format!("{e:?}"))?;
    mainloop.run();

    let st = st.borrow();
    // each click sent: the first hit after it, within half a period
    let mut lags: Vec<i64> = Vec::new();
    for &s in &st.sent {
        if let Some(&g) = st.got.iter().find(|&&g| g + 2000 >= s && g < s + period / 2) {
            lags.push(g as i64 - s as i64);
        }
    }
    lags.sort();
    if std::env::var_os("LAT_DEBUG").is_some() {
        eprintln!("sent {:?}\ngot  {:?}", &st.sent[..st.sent.len().min(5)], &st.got[..st.got.len().min(5)]);
    }
    if lags.is_empty() {
        return Err(format!("no clicks came back ({} sent, {} heard)", st.sent.len(), st.got.len()));
    }
    let ms = |f: i64| f as f64 * 1000.0 / RATE as f64;
    println!(
        "quantum {quantum}: {} clicks, latency min {:.2} ms, median {:.2} ms, max {:.2} ms ({}..{} frames)",
        lags.len(),
        ms(lags[0]),
        ms(lags[lags.len() / 2]),
        ms(*lags.last().unwrap()),
        lags[0],
        lags.last().unwrap()
    );
    Ok(())
}
