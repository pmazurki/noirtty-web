//! Terminal renderer with WebGPU and Canvas2D fallback

mod canvas2d;
#[cfg(web)]
mod webgpu;

pub use canvas2d::Canvas2DRenderer;
#[cfg(web)]
pub use webgpu::WebGpuRenderer;

use crate::terminal::Terminal;
use wasm_bindgen::prelude::*;

/// Renderer enum supporting WebGPU with Canvas2D fallback
pub enum Renderer {
    Canvas2D(Canvas2DRenderer),
    #[cfg(web)]
    WebGpu(WebGpuRenderer),
}

impl Renderer {
    /// Create a new renderer, preferring WebGPU with Canvas2D fallback
    pub async fn new(canvas_id: &str) -> Result<Self, JsValue> {
        #[cfg(web)]
        {
            // Try WebGPU first
            match WebGpuRenderer::new(canvas_id).await {
                Ok(renderer) => {
                    tracing::info!("Using WebGPU renderer");
                    return Ok(Renderer::WebGpu(renderer));
                }
                Err(e) => {
                    tracing::warn!("WebGPU not available: {:?}, falling back to Canvas2D", e);
                }
            }
        }

        let renderer = Canvas2DRenderer::new(canvas_id).await?;
        Ok(Renderer::Canvas2D(renderer))
    }

    /// Resize the renderer
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), JsValue> {
        match self {
            Renderer::Canvas2D(r) => r.resize(cols, rows),
            #[cfg(web)]
            Renderer::WebGpu(r) => r.resize(cols, rows),
        }
    }

    /// Update canvas dimensions from window resize
    pub fn set_size(&mut self, width: u32, height: u32) -> Result<(), JsValue> {
        match self {
            Renderer::Canvas2D(r) => r.set_size(width, height),
            #[cfg(web)]
            Renderer::WebGpu(r) => r.set_size(width, height),
        }
    }

    /// Configure renderer settings
    pub fn set_render_config(
        &mut self,
        font_size: f64,
        font_stack: &str,
        background: &str,
        selection: &str,
        cursor: &str,
        cursor_text: &str,
    ) -> Result<(), JsValue> {
        match self {
            Renderer::Canvas2D(r) => {
                r.set_render_config(font_size, font_stack, background, selection, cursor, cursor_text)
            }
            #[cfg(web)]
            Renderer::WebGpu(r) => {
                r.set_render_config(font_size, font_stack, background, selection, cursor, cursor_text)
            }
        }
    }

    /// Calculate columns and rows that fit in the given physical dimensions
    pub fn calculate_grid_size(&self, width: u32, height: u32) -> (u16, u16) {
        match self {
            Renderer::Canvas2D(r) => r.calculate_grid_size(width, height),
            #[cfg(web)]
            Renderer::WebGpu(r) => r.calculate_grid_size(width, height),
        }
    }

    /// Maximum surface dimension supported by the active renderer.
    pub fn max_surface_dim(&self) -> u32 {
        match self {
            Renderer::Canvas2D(r) => r.max_surface_dim(),
            #[cfg(web)]
            Renderer::WebGpu(r) => r.max_surface_dim(),
        }
    }

    /// Convert pixel coordinates to cell coordinates
    pub fn pixel_to_cell(&self, x: u32, y: u32) -> (u16, u16) {
        match self {
            Renderer::Canvas2D(r) => r.pixel_to_cell(x, y),
            #[cfg(web)]
            Renderer::WebGpu(r) => r.pixel_to_cell(x, y),
        }
    }

    /// Render the terminal
    pub fn render(&mut self, terminal: &Terminal) -> Result<(), JsValue> {
        match self {
            Renderer::Canvas2D(r) => r.render(terminal),
            #[cfg(web)]
            Renderer::WebGpu(r) => r.render(terminal),
        }
    }

    /// Get renderer type string
    pub fn renderer_type(&self) -> &'static str {
        match self {
            Renderer::Canvas2D(_) => "canvas2d",
            #[cfg(web)]
            Renderer::WebGpu(_) => "webgpu",
        }
    }

    /// Debug: number of text layout runs in the renderer.
    pub fn debug_text_runs(&self) -> u32 {
        match self {
            Renderer::Canvas2D(_) => 0,
            #[cfg(web)]
            Renderer::WebGpu(r) => r.debug_text_runs(),
        }
    }

    pub fn set_debug_text(&mut self, enabled: bool) {
        match self {
            Renderer::Canvas2D(_) => {}
            #[cfg(web)]
            Renderer::WebGpu(r) => r.set_debug_text(enabled),
        }
    }
}
