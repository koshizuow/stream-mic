//! Start with the computer: the program that is running now, with --hidden (to the tray, no window).
//! Linux: a desktop entry in ~/.config/autostart (XDG autostart, read by Cinnamon, GNOME, KDE and the rest).
//! Windows: a value "stream-mic" under HKCU\Software\Microsoft\Windows\CurrentVersion\Run.

#[cfg(not(windows))]
pub use xdg::{enabled, set, supported};
#[cfg(windows)]
pub use run_key::{enabled, set, supported};

/// An Exec argument, quoted the desktop entry way
#[cfg_attr(windows, allow(dead_code))]
fn quote(s: &str) -> String {
    let mut q = String::from("\"");
    for c in s.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            q.push('\\');
        }
        q.push(c);
    }
    q.push('"');
    q
}

#[cfg(not(windows))]
mod xdg {
    use std::path::PathBuf;

    /// Can this system do it
    pub fn supported() -> bool {
        cfg!(target_os = "linux") && file().is_some()
    }

    fn file() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("autostart").join("stream-mic.desktop"))
    }

    /// Is it on (our entry is there and not turned off by the desktop's own settings)
    pub fn enabled() -> bool {
        file().and_then(|f| std::fs::read_to_string(f).ok()).is_some_and(|t| !t.lines().any(|l| l.trim() == "Hidden=true" || l.trim() == "X-GNOME-Autostart-enabled=false"))
    }

    /// The program to start: an AppImage's own file (the running one sits in a temporary mount), else this executable
    fn program() -> std::io::Result<PathBuf> {
        match std::env::var_os("APPIMAGE") {
            Some(p) => Ok(PathBuf::from(p)),
            None => std::env::current_exe(),
        }
    }

    pub fn set(on: bool) -> Result<(), String> {
        let f = file().ok_or("no home folder")?;
        if !on {
            return match std::fs::remove_file(&f) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            };
        }
        let exe = program().map_err(|e| e.to_string())?;
        let text = format!(
            "[Desktop Entry]\nType=Application\nName=stream-mic\nComment=直播麥克風處理\nExec={} --hidden\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
            super::quote(&exe.to_string_lossy())
        );
        if let Some(d) = f.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        std::fs::write(&f, text).map_err(|e| e.to_string())
    }
}

#[cfg(windows)]
mod run_key {
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
    use windows::Win32::System::Registry::{RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ};

    const KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    const NAME: PCWSTR = w!("stream-mic");

    pub fn supported() -> bool {
        true
    }

    /// Is our value there (Task Manager's 啟動應用程式 switch keeps its own state elsewhere; turned off there it still
    /// reads as on here)
    pub fn enabled() -> bool {
        let mut size = 0u32;
        unsafe { RegGetValueW(HKEY_CURRENT_USER, KEY, NAME, RRF_RT_REG_SZ, None, None, Some(&mut size)) }.is_ok()
    }

    pub fn set(on: bool) -> Result<(), String> {
        if !on {
            let r = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, KEY, NAME) };
            return if r.is_ok() || r == ERROR_FILE_NOT_FOUND { Ok(()) } else { Err(format!("registry: {r:?}")) };
        }
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let cmd = format!("\"{}\" --hidden", exe.display());
        let wide: Vec<u16> = cmd.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes = unsafe { std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2) };
        let r = unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, KEY, NAME, REG_SZ.0, Some(bytes.as_ptr() as *const _), bytes.len() as u32) };
        if r.is_ok() { Ok(()) } else { Err(format!("registry: {r:?}")) }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn quotes_like_a_desktop_entry() {
        assert_eq!(super::quote("/a b/$x\"y"), "\"/a b/\\$x\\\"y\"");
    }
}
