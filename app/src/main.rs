//! The app: the engine (crates/engine) in this same process, its page in a window (WebView2 on Windows, WebKitGTK
//! on Linux), and an icon in the system tray. Closing the window leaves the engine running in the tray; "Quit" in
//! the tray's menu stops it. The menu follows the page's language (the settings', else the system's). `--hidden` starts in the tray only (start with the computer). Started a second time,
//! it asks the running one to show its window and exits. The engine's options pass through (--port, --fresh, ...).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::Write;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

fn show(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Another stream-mic has the port: ask it to show its window (POST {"show":true} to its page's API)
fn wake_other(port: u16) -> bool {
    let Ok(mut c) = std::net::TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_secs(1)) else {
        return false;
    };
    let body = r#"{"show":true}"#;
    write!(c, "POST /api/state HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).is_ok()
}

/// The tray menu's words: open, bypass, quit
fn words(lang: &str) -> [&'static str; 3] {
    if lang == "zh" { ["打開 stream-mic", "聽原音（暫停處理）", "結束 stream-mic"] } else { ["Open stream-mic", "Bypass (raw voice)", "Quit stream-mic"] }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let hidden = args.iter().any(|a| a == "--hidden");
    let engine = match stream_mic_engine::start(&args) {
        Ok(e) => e,
        Err(err) => {
            let port = args.iter().position(|a| a == "--port").and_then(|i| args.get(i + 1)).and_then(|p| p.parse().ok()).unwrap_or(8790);
            if wake_other(port) {
                return;
            }
            eprintln!("stream-mic: {err}");
            std::process::exit(1);
        }
    };
    engine.shared.in_app.store(true, Ordering::Relaxed);
    let (shared, port) = (engine.shared.clone(), engine.port);

    tauri::Builder::default()
        .setup(move |app| {
            let url = format!("http://127.0.0.1:{port}/").parse()?;
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("stream-mic")
                .inner_size(1120.0, 880.0)
                .min_inner_size(340.0, 440.0)
                .visible(!hidden)
                .build()?;

            let mut lang = stream_mic_engine::ui_lang(&shared);
            let [w_open, w_raw, w_quit] = words(lang);
            let open = MenuItem::with_id(app, "open", w_open, true, None::<&str>)?;
            let raw = CheckMenuItem::with_id(app, "raw", w_raw, true, false, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", w_quit, true, None::<&str>)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let menu = Menu::with_items(app, &[&open, &raw, &sep, &quit])?;
            let s = shared.clone();
            let mut tray = TrayIconBuilder::with_id("tray")
                .tooltip("stream-mic")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(move |app, e| match e.id().as_ref() {
                    "open" => show(app),
                    "raw" => {
                        let on = !s.cfg.lock().unwrap().bypass;
                        let _ = stream_mic_engine::http::apply(&s, &serde_json::json!({ "bypass": on }));
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                // a left click opens the window (Windows; Linux tray icons only have the menu)
                .on_tray_icon_event(|tray, e| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                        show(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;

            // the page and a second start ask for the window; the page also turns bypass on and off: keep the
            // menu's check mark in step, and its words with the language chosen in the page
            let (s, h, items) = (shared.clone(), app.handle().clone(), (open.clone(), raw.clone(), quit.clone()));
            std::thread::spawn(move || {
                let mut seen = s.show.load(Ordering::Acquire);
                let mut was = None;
                loop {
                    std::thread::sleep(Duration::from_millis(250));
                    let n = s.show.load(Ordering::Acquire);
                    if n != seen {
                        seen = n;
                        let h2 = h.clone();
                        let _ = h.run_on_main_thread(move || show(&h2));
                    }
                    let b = s.cfg.lock().unwrap().bypass;
                    if was != Some(b) {
                        was = Some(b);
                        let _ = raw.set_checked(b);
                    }
                    let l = stream_mic_engine::ui_lang(&s);
                    if l != lang {
                        lang = l;
                        let [a, b, c] = words(l);
                        let _ = items.0.set_text(a);
                        let _ = items.1.set_text(b);
                        let _ = items.2.set_text(c);
                    }
                }
            });
            Ok(())
        })
        .on_window_event(|w, e| {
            // closing the window keeps the engine (the stream goes on): it waits in the tray
            if let WindowEvent::CloseRequested { api, .. } = e {
                api.prevent_close();
                let _ = w.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("the app window did not start");
}
