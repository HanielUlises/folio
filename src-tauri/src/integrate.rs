//! First-run desktop integration.
//!
//! Registers Folio with the OS application menu so it can be found and launched
//! by name, without any separate installer — the moment the extracted binary is
//! run once, it shows up. Best-effort, idempotent and per-user (no elevation).

use std::path::Path;

/// Register the app in the OS launcher if not already done. Safe to call every
/// startup: it only writes when something is missing or has changed.
pub fn ensure_registered() {
    let Ok(exe) = std::env::current_exe() else { return };
    #[cfg(all(unix, not(target_os = "macos")))]
    linux_register(&exe);
    #[cfg(target_os = "windows")]
    windows_register(&exe);
    let _ = exe;
}

/// Write `bytes` to `path` when absent or a different size (icon upgrades).
#[cfg(any(all(unix, not(target_os = "macos")), target_os = "windows"))]
fn write_if_changed(path: &Path, bytes: &[u8]) {
    let current = std::fs::metadata(path).map(|m| m.len() == bytes.len() as u64).unwrap_or(false);
    if !current {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, bytes);
    }
}

/// Install a `.desktop` entry and hicolor icons under `~/.local/share`.
#[cfg(all(unix, not(target_os = "macos")))]
fn linux_register(exe: &Path) {
    let Some(data) = dirs::data_dir() else { return };
    let exec = exe.display().to_string();
    let entry = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Version=1.0\n\
         Name=Folio\n\
         GenericName=PDF Reader\n\
         Comment=PDF reader and library manager\n\
         Exec={exec}\n\
         Icon=folio\n\
         Terminal=false\n\
         Categories=Office;Viewer;\n\
         StartupWMClass=folio\n\
         Keywords=PDF;reader;library;documents;\n"
    );
    let desktop = data.join("applications").join("folio.desktop");
    // Rewrite when missing or when the executable moved to a new path.
    let up_to_date = std::fs::read_to_string(&desktop)
        .map(|s| s.contains(&format!("Exec={exec}\n")))
        .unwrap_or(false);
    if !up_to_date {
        if let Some(parent) = desktop.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&desktop, entry);
    }

    let hicolor = data.join("icons").join("hicolor");
    write_if_changed(&hicolor.join("128x128/apps/folio.png"), include_bytes!("../icons/128x128.png"));
    write_if_changed(&hicolor.join("256x256/apps/folio.png"), include_bytes!("../icons/128x128@2x.png"));
    write_if_changed(&hicolor.join("32x32/apps/folio.png"), include_bytes!("../icons/32x32.png"));
}

/// Create a Start Menu shortcut so Folio appears in Windows search.
#[cfg(target_os = "windows")]
fn windows_register(exe: &Path) {
    let Some(appdata) = dirs::data_dir() else { return }; // %APPDATA%\Roaming
    let lnk = appdata
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join("Folio.lnk");
    if lnk.exists() {
        return;
    }
    if let Some(parent) = lnk.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let target = exe.display().to_string();
    let lnk_s = lnk.display().to_string();
    // Build the .lnk via the WScript.Shell COM object; the exe's embedded icon
    // (from build.rs) is used for the shortcut.
    let ps = format!(
        "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{lnk_s}'); \
         $s.TargetPath='{target}'; $s.IconLocation='{target},0'; \
         $s.Description='PDF reader and library manager'; $s.Save()"
    );
    let _ = std::process::Command::new("powershell")
        .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &ps])
        .spawn();
}
