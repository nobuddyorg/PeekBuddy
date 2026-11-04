//! The renderer (#65, #252, ADR 0005): glow on x64, wgpu on DX12 on arm64.
//!
//! x64 renders through glow (OpenGL), which needs far less memory than wgpu;
//! arm64 has no OpenGL driver and always uses wgpu. The `wgpu` feature moves
//! x64 to wgpu as well, for A/B runs.
//!
//! eframe's wgpu defaults read `WGPU_BACKEND`, `WGPU_POWER_PREF` and the
//! `WGPU_*` instance flags from the environment, which would let a stray
//! variable move a production viewer to Vulkan (Adreno's Vulkan driver is
//! broken, wgpu#10621) or to GL (absent on Snapdragon). The wgpu instance is
//! therefore built explicitly, and the environment is not consulted.

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

#[cfg(any(target_arch = "aarch64", feature = "wgpu"))]
pub use self::wgpu_renderer::{adapter, native_options};

#[cfg(not(any(target_arch = "aarch64", feature = "wgpu")))]
pub use self::glow_renderer::{adapter, native_options};

#[cfg(any(target_arch = "aarch64", feature = "wgpu"))]
mod wgpu_renderer {
    use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew, wgpu};

    use super::Adapter;

    /// The wgpu setup: DX12 only, no debug or validation flags, backend
    /// options at their defaults, the low-power adapter on machines with two
    /// GPUs.
    #[must_use]
    pub fn setup() -> WgpuSetupCreateNew {
        let mut create = WgpuSetupCreateNew::without_display_handle();
        create.instance_descriptor.backends = wgpu::Backends::DX12;
        create.instance_descriptor.flags = wgpu::InstanceFlags::empty();
        create.instance_descriptor.backend_options = wgpu::BackendOptions::default();
        create.power_preference = wgpu::PowerPreference::LowPower;
        create
    }

    /// The eframe options of the viewer window.
    #[must_use]
    pub fn native_options(viewport: egui::ViewportBuilder) -> eframe::NativeOptions {
        eframe::NativeOptions {
            viewport,
            renderer: eframe::Renderer::Wgpu,
            wgpu_options: WgpuConfiguration {
                wgpu_setup: WgpuSetup::CreateNew(setup()),
                ..WgpuConfiguration::default()
            },
            // Must stay true: with false, eframe calls `process::exit(0)` on
            // an init error and the error is lost.
            run_and_return: true,
            ..eframe::NativeOptions::default()
        }
    }

    /// The adapter of a running frame.
    #[must_use]
    pub fn adapter(frame: &eframe::Frame) -> Option<Adapter> {
        let state = frame.wgpu_render_state()?;
        let info = state.adapter.get_info();
        Some(Adapter {
            name: info.name,
            backend: format!("{:?}", info.backend),
            device_type: format!("{:?}", info.device_type),
        })
    }

    #[cfg(test)]
    mod tests {
        use eframe::egui_wgpu::wgpu;

        use super::setup;

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
                    "gpu::wgpu_renderer::tests::the_instance_is_dx12_only_whatever_the_environment_says",
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
            // The child really ran the test (a filter that matches nothing
            // also exits 0).
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("1 passed"),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
            // And the parent, with a clean environment, agrees.
            assert_eq!(setup().instance_descriptor.backends, wgpu::Backends::DX12);
        }
    }
}

#[cfg(not(any(target_arch = "aarch64", feature = "wgpu")))]
mod glow_renderer {
    use super::Adapter;

    /// The eframe options of the viewer window.
    #[must_use]
    pub fn native_options(viewport: egui::ViewportBuilder) -> eframe::NativeOptions {
        eframe::NativeOptions {
            viewport,
            renderer: eframe::Renderer::Glow,
            // Must stay true: with false, eframe calls `process::exit(0)` on
            // an init error and the error is lost.
            run_and_return: true,
            ..eframe::NativeOptions::default()
        }
    }

    /// The adapter of a running frame: the OpenGL renderer string.
    #[must_use]
    pub fn adapter(frame: &eframe::Frame) -> Option<Adapter> {
        let gl = frame.gl()?;
        let renderer = peekbuddy_win::gl::renderer_string(gl);
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

    /// Microsoft's OpenGL 1.1 fallback and Mesa's software rasterisers: no
    /// driver with real OpenGL is installed.
    fn is_software_renderer(renderer: &str) -> bool {
        let lower = renderer.to_ascii_lowercase();
        ["gdi generic", "llvmpipe", "softpipe"]
            .iter()
            .any(|software| lower.contains(software))
    }

    #[cfg(test)]
    mod tests {
        use super::is_software_renderer;

        #[test]
        fn software_opengl_is_told_from_a_driver() {
            assert!(is_software_renderer("GDI Generic"));
            assert!(is_software_renderer("llvmpipe (LLVM 15.0.7, 256 bits)"));
            assert!(!is_software_renderer("Intel(R) UHD Graphics"));
        }
    }
}
