use std::rc::Rc;
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_core_foundation::CGSize;
use objc2_metal::{
    MTLCommandBuffer, MTLCommandQueue, MTLCreateSystemDefaultDevice, MTLDevice, MTLDrawable,
    MTLPixelFormat,
};
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
use skia_safe::gpu::{self, DirectContext, SurfaceOrigin, backend_render_targets, mtl};
use skia_safe::{ColorSpace, ColorType};
use winit::dpi::PhysicalSize;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

use super::export_png::skia_color_space;
use super::{RenderFrame, Renderer, render_canvas};
use crate::app::{AppConfig, MacosColorSpace};
use crate::editor::Editor;
use crate::layout::{ViewportOffset, layout_metrics};
use crate::macos::color_space_for_config;
use crate::model::Coord;
use crate::perf::FrameTiming;

pub(super) struct MetalRenderer {
    layer: Retained<CAMetalLayer>,
    _device: Retained<ProtocolObject<dyn MTLDevice>>,
    command_queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    skia: DirectContext,
    color_space: ColorSpace,
}

impl MetalRenderer {
    pub(super) fn new(window: &Rc<Window>, color_space: MacosColorSpace) -> Result<Self> {
        let skia_color_space = skia_color_space(color_space)?;
        let device = MTLCreateSystemDefaultDevice().context("no Metal device is available")?;
        let command_queue = device
            .newCommandQueue()
            .context("failed to create Metal command queue")?;
        let layer = CAMetalLayer::new();
        layer.setDevice(Some(&device));
        layer.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
        layer.setPresentsWithTransaction(false);
        layer.setFramebufferOnly(false);
        layer.setWantsExtendedDynamicRangeContent(false);
        Self::apply_layer_color_space(&layer, color_space)?;
        Self::resize_layer(&layer, window.inner_size(), window.scale_factor());

        let backend = unsafe {
            mtl::BackendContext::new(
                Retained::as_ptr(&device) as mtl::Handle,
                Retained::as_ptr(&command_queue) as mtl::Handle,
            )
        };
        let skia = gpu::direct_contexts::make_metal(&backend, None)
            .context("failed to create Skia Metal context")?;

        let handle = window
            .window_handle()
            .map_err(|error| anyhow!("failed to get raw window handle: {error}"))?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return Err(anyhow!("expected AppKit window handle on macOS"));
        };
        let view = unsafe { &*(handle.ns_view.as_ptr().cast::<objc2_app_kit::NSView>()) };
        view.setWantsLayer(true);
        view.setLayer(Some(&layer.clone().into_super()));

        Ok(Self {
            layer,
            _device: device,
            command_queue,
            skia,
            color_space: skia_color_space,
        })
    }

    pub(super) fn resize(&self, size: PhysicalSize<u32>, scale_factor: f64) {
        Self::resize_layer(&self.layer, size, scale_factor);
    }

    pub(super) fn set_color_space(&mut self, color_space: MacosColorSpace) -> Result<()> {
        Self::apply_layer_color_space(&self.layer, color_space)?;
        self.color_space = skia_color_space(color_space)?;
        Ok(())
    }

    pub(super) fn render(
        &mut self,
        window: &Window,
        state: &Editor,
        content: &[Coord],
        renderer: &Renderer,
        config: &AppConfig,
        viewport: ViewportOffset,
        toolbar_hotspot_hovered: bool,
    ) -> Result<FrameTiming> {
        let size = window.inner_size();
        let width = size.width.max(1) as usize;
        let height = size.height.max(1) as usize;
        let metrics = renderer.metrics(window.scale_factor());
        let title_metrics = renderer.title_metrics(window.scale_factor());

        let buffer_started = Instant::now();
        let Some(drawable) = self.layer.nextDrawable() else {
            return Ok(FrameTiming::default());
        };
        let buffer_acquisition = buffer_started.elapsed();

        let raster_started = Instant::now();
        let texture_info =
            unsafe { mtl::TextureInfo::new(Retained::as_ptr(&drawable.texture()) as mtl::Handle) };
        let backend_render_target =
            backend_render_targets::make_mtl((width as i32, height as i32), &texture_info);
        let mut surface = gpu::surfaces::wrap_backend_render_target(
            &mut self.skia,
            &backend_render_target,
            SurfaceOrigin::TopLeft,
            ColorType::BGRA8888,
            Some(self.color_space.clone()),
            None,
        )
        .context("failed to wrap Metal drawable as a Skia surface")?;
        let breakdown = render_canvas(
            surface.canvas(),
            state,
            config,
            RenderFrame {
                metrics: &metrics,
                toolbar_metrics: &title_metrics,
                layout: layout_metrics(
                    width,
                    height,
                    &metrics,
                    (title_metrics.cell_width, title_metrics.cell_height),
                    &state.toolbar,
                    config.transparent_menubar,
                    window.scale_factor(),
                ),
                width,
                viewport,
                toolbar_hotspot_hovered,
                content,
                toolbar_cache: &renderer.toolbar_cache,
            },
        );
        self.skia.flush_and_submit();
        drop(surface);
        let rasterization = raster_started.elapsed();

        let presentation_started = Instant::now();
        let command_buffer = self
            .command_queue
            .commandBuffer()
            .context("failed to create Metal presentation command buffer")?;
        let drawable: Retained<ProtocolObject<dyn MTLDrawable>> = (&drawable).into();
        command_buffer.presentDrawable(&drawable);
        command_buffer.commit();

        Ok(FrameTiming {
            buffer_acquisition,
            rasterization,
            presentation: presentation_started.elapsed(),
            toolbar: breakdown.toolbar,
            grid: breakdown.grid,
            minimap: breakdown.minimap,
        })
    }

    fn resize_layer(layer: &CAMetalLayer, size: PhysicalSize<u32>, scale_factor: f64) {
        layer.setContentsScale(scale_factor);
        layer.setDrawableSize(CGSize::new(
            size.width.max(1) as f64,
            size.height.max(1) as f64,
        ));
    }

    fn apply_layer_color_space(layer: &CAMetalLayer, color_space: MacosColorSpace) -> Result<()> {
        let ns_color_space = color_space_for_config(color_space);
        let cg_color_space = ns_color_space
            .CGColorSpace()
            .context("configured macOS color space has no Core Graphics representation")?;
        layer.setColorspace(Some(&cg_color_space));
        Ok(())
    }
}
