//! The OpenGL renderer string of the viewer's glow context (#252).

use glow::HasContext;

/// What the driver calls its renderer (`GL_RENDERER`), for example
/// `Intel(R) UHD Graphics` or `GDI Generic` when no OpenGL driver is
/// installed.
#[must_use]
pub fn renderer_string(gl: &glow::Context) -> String {
    // SAFETY: `GL_RENDERER` is a valid parameter name for every OpenGL
    // version; the call only reads a string of the current context, and
    // eframe makes the context current while it calls the application.
    unsafe { gl.get_parameter_string(glow::RENDERER) }
}
