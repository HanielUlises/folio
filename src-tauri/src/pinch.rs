//! Linux trackpad pinch-to-zoom via libinput.
//!
//! winit — and therefore eframe — only emits `WindowEvent::PinchGesture` on
//! macOS/iOS; on X11 and Wayland a two-finger pinch never reaches the app, so
//! `egui`'s `zoom_delta()` stays at 1.0 and the gesture does nothing. To make
//! pinch work on Linux we read the gesture straight from libinput in a
//! background thread and forward each *incremental* scale factor to the UI,
//! which applies it exactly like a native `zoom_delta` (anchored under the
//! cursor — see `Folio::apply_scroll_zoom`).
//!
//! Reading libinput needs access to `/dev/input/event*`. The user's account
//! must be in the `input` group (or the app run under a logind seat). If access
//! is denied the thread simply exits and Ctrl+scroll zoom keeps working — the
//! rest of the app is unaffected.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::mpsc::{Receiver, Sender};

use input::event::gesture::{GestureEvent, GesturePinchEvent, GesturePinchEventTrait};
use input::event::Event;
use input::{Libinput, LibinputInterface};

/// Opens the raw evdev device files libinput asks for. libinput passes the exact
/// open flags it wants (read-only for most, read-write for force-feedback etc.),
/// so we honour them verbatim rather than assuming.
struct Interface;

impl LibinputInterface for Interface {
    fn open_restricted(&mut self, path: &Path, flags: i32) -> Result<OwnedFd, i32> {
        let cpath = match std::ffi::CString::new(path.as_os_str().as_bytes()) {
            Ok(p) => p,
            Err(_) => return Err(libc::EINVAL),
        };
        // SAFETY: `cpath` is a valid NUL-terminated string for the duration of
        // the call; on success we own the returned fd and wrap it in `OwnedFd`.
        let fd = unsafe { libc::open(cpath.as_ptr(), flags) };
        if fd < 0 {
            Err(unsafe { *libc::__errno_location() })
        } else {
            Ok(unsafe { OwnedFd::from_raw_fd(fd) })
        }
    }

    fn close_restricted(&mut self, fd: OwnedFd) {
        drop(fd); // OwnedFd closes the descriptor on drop
    }
}

/// Handle to the background pinch reader. Drained once per frame by the UI.
pub struct PinchListener {
    rx: Receiver<f32>,
}

impl PinchListener {
    /// Spawn the libinput reader thread. `ctx` is cloned so the thread can wake
    /// the UI (request a repaint) the moment a pinch arrives. Always returns a
    /// handle; if libinput can't be initialised the thread just exits and every
    /// `take_factor()` call yields the no-op factor `1.0`.
    pub fn spawn(ctx: egui::Context) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<f32>();
        let _ = std::thread::Builder::new()
            .name("folio-pinch".into())
            .spawn(move || run(tx, ctx));
        Self { rx }
    }

    /// Product of every pinch factor received since the last call — i.e. the
    /// cumulative multiplicative zoom for this frame. Returns `1.0` when no
    /// pinch happened. Multiplying folds any number of libinput updates that
    /// landed between frames into a single, correct zoom step.
    pub fn take_factor(&self) -> f32 {
        let mut factor = 1.0_f32;
        while let Ok(f) = self.rx.try_recv() {
            factor *= f;
        }
        factor
    }
}

/// True if at least one `/dev/input/event*` node is readable by this process.
/// libinput prints a "permission denied" warning per device when it can't open
/// one, so we check up front and bail quietly when the user isn't in the `input`
/// group — no spam, and Ctrl+scroll zoom still works.
fn have_device_access() -> bool {
    let Ok(dir) = std::fs::read_dir("/dev/input") else {
        return false;
    };
    for entry in dir.flatten() {
        if !entry.file_name().to_string_lossy().starts_with("event") {
            continue;
        }
        if let Ok(c) = std::ffi::CString::new(entry.path().as_os_str().as_bytes()) {
            // SAFETY: `c` is a valid NUL-terminated path; access() only reads it.
            if unsafe { libc::access(c.as_ptr(), libc::R_OK) } == 0 {
                return true;
            }
        }
    }
    false
}

fn run(tx: Sender<f32>, ctx: egui::Context) {
    if !have_device_access() {
        return; // not in the `input` group — skip without libinput warnings
    }
    let mut li = Libinput::new_with_udev(Interface);
    if li.udev_assign_seat("seat0").is_err() {
        return; // no seat / no permission — give up silently
    }
    let fd: RawFd = li.as_raw_fd();

    // libinput reports the pinch scale as an absolute value relative to gesture
    // start (1.0 at Begin). The UI wants an incremental factor per step, so we
    // track the previous absolute scale and forward the ratio.
    let mut last_scale = 1.0_f64;

    loop {
        // Block until libinput has readable data rather than busy-polling.
        let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        let r = unsafe { libc::poll(&mut pfd, 1, -1) };
        if r < 0 {
            match unsafe { *libc::__errno_location() } {
                libc::EINTR => continue,
                _ => return,
            }
        }
        if li.dispatch().is_err() {
            return;
        }
        for event in &mut li {
            let Event::Gesture(GestureEvent::Pinch(pinch)) = event else {
                continue;
            };
            match &pinch {
                GesturePinchEvent::Begin(_) | GesturePinchEvent::End(_) => {
                    last_scale = 1.0;
                }
                GesturePinchEvent::Update(_) => {
                    let scale = pinch.scale();
                    if scale <= 0.0 || last_scale <= 0.0 {
                        continue;
                    }
                    let factor = (scale / last_scale) as f32;
                    last_scale = scale;
                    if factor.is_finite() && (factor - 1.0).abs() > 1e-4 {
                        // Receiver gone → UI shut down; stop the thread.
                        if tx.send(factor).is_err() {
                            return;
                        }
                        ctx.request_repaint();
                    }
                }
                _ => {} // GesturePinchEvent is #[non_exhaustive]
            }
        }
    }
}
