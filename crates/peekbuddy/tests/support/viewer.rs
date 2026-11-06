//! Closing the viewer an engine started (#65, ADR 0005).
#![cfg(windows)]

use std::path::Path;
use std::time::{Duration, Instant};

use peekbuddy_win::window::WindowHandle;

/// Waits for the viewer an engine of `data` started, closes it, and returns
/// once no window of it has come back for a few seconds.
///
/// On a machine without OpenGL 2.0 the viewer opens a window for glow, gives it
/// up and opens another one for wgpu: closing the first leaves the second, so
/// the window is closed again until none is left.
pub fn close_viewer(data: &Path) {
    let title = peekbuddy_viewer::isolated_title(peekbuddy_win::pipe::stable_hash(
        &data.to_string_lossy().to_lowercase(),
    ));
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut absent_since = None;
    let mut closed = false;
    loop {
        if let Some(shown) = WindowHandle::find_title(&title) {
            absent_since = None;
            closed = shown.post_close().is_ok() || closed;
        } else if closed {
            let since = *absent_since.get_or_insert_with(Instant::now);
            if since.elapsed() > Duration::from_secs(5) {
                return;
            }
        }
        assert!(
            Instant::now() < deadline,
            "the viewer window did not show up or did not close (closed once: {closed}); \
             viewer.log: {:?}",
            std::fs::read_to_string(data.join("logs").join("viewer.log")).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}
