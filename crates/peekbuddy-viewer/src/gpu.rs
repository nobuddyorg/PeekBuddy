//! The renderer (#65, #252, ADR 0005): glow, then wgpu on x64; wgpu on arm64.
//!
//! x64 tries glow (OpenGL) first, which needs far less memory than wgpu, and
//! falls back to wgpu with DX12 when the machine has no OpenGL 2.0 driver
//! (Remote Desktop without GPU acceleration, a bare VM, a hosted CI runner).
//! arm64 has no OpenGL driver and only has wgpu. The `wgpu` feature drops glow
//! on x64 too, for A/B runs.
//!
//! eframe's wgpu defaults read `WGPU_BACKEND`, `WGPU_POWER_PREF` and the
//! `WGPU_*` instance flags from the environment, which would let a stray
//! variable move a production viewer to Vulkan (Adreno's Vulkan driver is
//! broken, wgpu#10621) or to GL (absent on Snapdragon). The wgpu instance is
//! therefore built explicitly, and the environment is not consulted.

use std::fmt;

use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew, wgpu};

/// A renderer the viewer can start with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Renderer {
    /// OpenGL through glow (x64 only).
    #[cfg(all(target_arch = "x86_64", not(feature = "wgpu")))]
    Glow,
    /// DX12 through wgpu.
    Wgpu,
}

impl fmt::Display for Renderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            #[cfg(all(target_arch = "x86_64", not(feature = "wgpu")))]
            Self::Glow => "glow (OpenGL)",
            Self::Wgpu => "wgpu (DX12)",
        })
    }
}

/// The renderers to try, in order of preference.
#[must_use]
pub const fn renderers() -> &'static [Renderer] {
    #[cfg(all(target_arch = "x86_64", not(feature = "wgpu")))]
    {
        &[Renderer::Glow, Renderer::Wgpu]
    }
    #[cfg(not(all(target_arch = "x86_64", not(feature = "wgpu"))))]
    {
        &[Renderer::Wgpu]
    }
}

/// The adapter the viewer ended up on, for the startup log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adapter {
    /// The adapter name (wgpu) or the OpenGL renderer string (glow).
    pub name: String,
    /// The backend: `Dx12` (wgpu) or `OpenGL` (glow).
    pub backend: String,
    /// The device type: `IntegratedGpu`, `DiscreteGpu`, `Cpu` (WARP or a
    /// software OpenGL), ...
    pub device_type: String,
}

impl Adapter {
    /// Software rendering (WARP, GDI Generic): slow and a sign of a missing
    /// driver.
    #[must_use]
    pub fn is_software(&self) -> bool {
        self.device_type == "Cpu"
    }
}

/// The wgpu setup: DX12 only, no debug or validation flags, backend options
/// at their defaults, the low-power adapter on machines with two GPUs.
#[must_use]
pub fn setup() -> WgpuSetupCreateNew {
    let mut create = WgpuSetupCreateNew::without_display_handle();
    create.instance_descriptor.backends = wgpu::Backends::DX12;
    create.instance_descriptor.flags = wgpu::InstanceFlags::empty();
    create.instance_descriptor.backend_options = wgpu::BackendOptions::default();
    create.power_preference = wgpu::PowerPreference::LowPower;
    create
}

/// The eframe options of the viewer window for `renderer`.
#[must_use]
pub fn native_options(
    renderer: Renderer,
    viewport: egui::ViewportBuilder,
) -> eframe::NativeOptions {
    let options = eframe::NativeOptions {
        viewport,
        // Must stay true: with false, eframe calls `process::exit(0)` on an
        // init error and the error is lost (and no fallback is possible).
        run_and_return: true,
        ..eframe::NativeOptions::default()
    };
    match renderer {
        #[cfg(all(target_arch = "x86_64", not(feature = "wgpu")))]
        Renderer::Glow => eframe::NativeOptions {
            renderer: eframe::Renderer::Glow,
            ..options
        },
        Renderer::Wgpu => eframe::NativeOptions {
            renderer: eframe::Renderer::Wgpu,
            wgpu_options: WgpuConfiguration {
                wgpu_setup: WgpuSetup::CreateNew(setup()),
                ..WgpuConfiguration::default()
            },
            ..options
        },
    }
}

/// The adapter of a running frame.
#[must_use]
pub fn adapter(frame: &eframe::Frame) -> Option<Adapter> {
    if let Some(state) = frame.wgpu_render_state() {
        let info = state.adapter.get_info();
        return Some(Adapter {
            name: info.name,
            backend: format!("{:?}", info.backend),
            device_type: format!("{:?}", info.device_type),
        });
    }
    #[cfg(all(target_arch = "x86_64", not(feature = "wgpu")))]
    {
        let renderer = peekbuddy_win::gl::renderer_string(frame.gl()?);
        let device_type = if is_software_renderer(&renderer) {
            "Cpu"
        } else {
            "Gpu"
        };
        Some(Adapter {
            name: renderer,
            backend: String::from("OpenGL"),
            device_type: String::from(device_type),
        })
    }
    #[cfg(not(all(target_arch = "x86_64", not(feature = "wgpu"))))]
    {
        None
    }
}

/// Microsoft's OpenGL 1.1 fallback and Mesa's software rasterisers: no
/// driver with real OpenGL is installed.
#[cfg(any(test, all(target_arch = "x86_64", not(feature = "wgpu"))))]
fn is_software_renderer(renderer: &str) -> bool {
    let lower = renderer.to_ascii_lowercase();
    ["gdi generic", "llvmpipe", "softpipe"]
        .iter()
        .any(|software| lower.contains(software))
}

#[cfg(test)]
mod tests {
    use eframe::egui_wgpu::wgpu;

    use super::{Renderer, is_software_renderer, renderers, setup};

    #[test]
    fn the_instance_is_dx12_only_whatever_the_environment_says() {
        // Re-run this very test with the variables a user might have set.
        if std::env::var_os("PEEKBUDDY_TEST_CHILD").is_some() {
            let create = setup();
            assert_eq!(create.instance_descriptor.backends, wgpu::Backends::DX12);
            assert_eq!(
                create.instance_descriptor.flags,
                wgpu::InstanceFlags::empty()
            );
            assert_eq!(create.power_preference, wgpu::PowerPreference::LowPower);
            return;
        }
        let exe = std::env::current_exe().unwrap();
        let output = std::process::Command::new(exe)
            .args([
                "--exact",
                "gpu::tests::the_instance_is_dx12_only_whatever_the_environment_says",
                "--nocapture",
            ])
            .env("PEEKBUDDY_TEST_CHILD", "1")
            .env("WGPU_BACKEND", "vulkan")
            .env("WGPU_POWER_PREF", "high")
            .env("WGPU_ADAPTER_NAME", "nothing")
            .env("WGPU_VALIDATION", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        // The child really ran the test (a filter that matches nothing also
        // exits 0).
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        // And the parent, with a clean environment, agrees.
        assert_eq!(setup().instance_descriptor.backends, wgpu::Backends::DX12);
    }

    #[test]
    fn wgpu_is_always_the_last_renderer_to_try() {
        let list = renderers();
        assert_eq!(list.last(), Some(&Renderer::Wgpu));
        #[cfg(all(target_arch = "x86_64", not(feature = "wgpu")))]
        assert_eq!(list.first(), Some(&Renderer::Glow));
        #[cfg(not(all(target_arch = "x86_64", not(feature = "wgpu"))))]
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn software_opengl_is_told_from_a_driver() {
        assert!(is_software_renderer("GDI Generic"));
        assert!(is_software_renderer("llvmpipe (LLVM 15.0.7, 256 bits)"));
        assert!(!is_software_renderer("Intel(R) UHD Graphics"));
    }
}
