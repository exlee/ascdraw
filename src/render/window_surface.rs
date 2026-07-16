use std::rc::Rc;

use anyhow::{Result, anyhow};
use softbuffer::{Context as SoftContext, Surface};
use winit::dpi::PhysicalSize;
use winit::window::Window;

use super::{Renderer, render, resize_surface};
use crate::app::AppConfig;
#[cfg(target_os = "macos")]
use crate::diagnostics::log_error;
use crate::editor::Editor;
use crate::layout::ViewportOffset;
use crate::perf::FrameTiming;

pub struct WindowSurface {
    backend: Backend,
}

enum Backend {
    #[cfg(target_os = "macos")]
    Metal(super::metal::MetalRenderer),
    Softbuffer(Surface<Rc<Window>, Rc<Window>>),
}

impl WindowSurface {
    pub fn new(window: &Rc<Window>, _config: &AppConfig) -> Result<Self> {
        #[cfg(target_os = "macos")]
        match super::metal::MetalRenderer::new(window, _config.macos.color_space) {
            Ok(renderer) => {
                return Ok(Self {
                    backend: Backend::Metal(renderer),
                });
            }
            Err(error) => {
                log_error(format!(
                    "Metal renderer initialization failed; using softbuffer: {error:#}"
                ));
            }
        }

        let context =
            SoftContext::new(window.clone()).map_err(|error| anyhow!(error.to_string()))?;
        let mut surface =
            Surface::new(&context, window.clone()).map_err(|error| anyhow!(error.to_string()))?;
        resize_surface(&mut surface, window.inner_size())?;
        Ok(Self {
            backend: Backend::Softbuffer(surface),
        })
    }

    pub fn resize(&mut self, _window: &Window, size: PhysicalSize<u32>) -> Result<()> {
        match &mut self.backend {
            #[cfg(target_os = "macos")]
            Backend::Metal(renderer) => {
                renderer.resize(size, _window.scale_factor());
                Ok(())
            }
            Backend::Softbuffer(surface) => resize_surface(surface, size),
        }
    }

    pub fn apply_config(&mut self, _config: &AppConfig) -> Result<()> {
        match &mut self.backend {
            #[cfg(target_os = "macos")]
            Backend::Metal(renderer) => renderer.set_color_space(_config.macos.color_space),
            Backend::Softbuffer(_) => Ok(()),
        }
    }

    pub fn render(
        &mut self,
        window: &Window,
        state: &Editor,
        renderer: &Renderer,
        config: &AppConfig,
        viewport: ViewportOffset,
    ) -> Result<FrameTiming> {
        match &mut self.backend {
            #[cfg(target_os = "macos")]
            Backend::Metal(surface) => surface.render(window, state, renderer, config, viewport),
            Backend::Softbuffer(surface) => {
                render(window, surface, state, renderer, config, viewport)
            }
        }
    }
}
