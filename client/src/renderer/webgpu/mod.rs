//! WebGPU terminal renderer

mod buffers;
mod pipeline;

use crate::terminal::Terminal;
use buffers::{CellInstance, GridUniforms};
use glyphon::{
    Attrs, Buffer, Cache, Color, ColorMode, Family, FontSystem, Metrics, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, Wrap,
};
use pipeline::BackgroundPipeline;
use std::sync::Arc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::HtmlCanvasElement;
use js_sys::Reflect;

/// WebGPU-based terminal renderer
pub struct WebGpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    canvas: HtmlCanvasElement,

    // Rendering state
    background_pipeline: BackgroundPipeline,
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,

    // Text rendering
    font_system: FontSystem,
    swash_cache: SwashCache,
    text_atlas: TextAtlas,
    text_renderer: TextRenderer,
    text_buffer: Buffer,
    viewport: Viewport,

    // Grid state
    cols: u16,
    rows: u16,
    cell_width: f64,
    cell_height: f64,
    font_size: f64,
    dpr: f64,

    // Colors
    background_color: [f32; 4],
    selection_color: [f32; 4],
    cursor_color: [f32; 4],
    cursor_text_color: [f32; 4],
    cursor_text_color_u8: [u8; 3],
    default_fg: [u8; 3],
    font_family: FontFamily,
    frame_counter: u64,
    last_text_runs: u32,
    debug_text: bool,
    max_surface_dim: u32,
}

impl WebGpuRenderer {
    /// Create a new WebGPU renderer
    pub async fn new(canvas_id: &str) -> Result<Self, JsValue> {
        const GL_MAX_SURFACE_DIM: u32 = 2048;
        let window = web_sys::window().ok_or("No window")?;
        let document = window.document().ok_or("No document")?;
        let canvas: HtmlCanvasElement = document
            .get_element_by_id(canvas_id)
            .ok_or("Canvas not found")?
            .dyn_into()?;

        let device_dpr = window.device_pixel_ratio();
        let rect = canvas.get_bounding_client_rect();
        let raw_width = (rect.width() * device_dpr) as u32;
        let raw_height = (rect.height() * device_dpr) as u32;
        let logical_width = rect.width() as f32;
        let logical_height = rect.height() as f32;

        // Create wgpu instance (auto-detect, prefer GL unless WebGPU is forced)
        let force_webgpu = should_force_webgpu();
        let force_gl_flag = should_force_gl();
        let has_webgpu = browser_has_webgpu();
        let prefer_gl = !force_webgpu && is_apple_safari();

        let mut used_gl = false;
        let mut reason = " (fallback)";
        let (surface, adapter) = if force_webgpu {
            create_surface_and_adapter(&canvas, wgpu::Backends::BROWSER_WEBGPU).await?
        } else if force_gl_flag {
            used_gl = true;
            reason = " (forced)";
            create_surface_and_adapter(&canvas, wgpu::Backends::GL).await?
        } else if prefer_gl {
            match create_surface_and_adapter(&canvas, wgpu::Backends::GL).await {
                Ok(pair) => {
                    used_gl = true;
                    reason = " (iOS Safari fallback)";
                    pair
                }
                Err(_) => create_surface_and_adapter(&canvas, wgpu::Backends::BROWSER_WEBGPU).await?,
            }
        } else if !has_webgpu {
            used_gl = true;
            reason = " (no WebGPU support)";
            create_surface_and_adapter(&canvas, wgpu::Backends::GL).await?
        } else {
            match create_surface_and_adapter(&canvas, wgpu::Backends::BROWSER_WEBGPU).await {
                Ok(pair) => pair,
                Err(_) => {
                    used_gl = true;
                    reason = " (webgpu init failed)";
                    create_surface_and_adapter(&canvas, wgpu::Backends::GL).await?
                }
            }
        };

        if force_webgpu && used_gl {
            tracing::warn!("WebGPU: forced WebGPU failed, falling back to GL");
        }

        if used_gl {
            tracing::warn!("WebGPU: using GL backend{}", reason);
        }

        log_adapter_info(&adapter, &surface);
        let adapter_max = adapter.limits().max_texture_dimension_2d;
        let max_surface_dim = if used_gl && is_apple_safari() {
            GL_MAX_SURFACE_DIM.min(adapter_max)
        } else {
            adapter_max
        };
        let width = raw_width.min(max_surface_dim).max(1);
        let height = raw_height.min(max_surface_dim).max(1);
        canvas.set_width(width);
        canvas.set_height(height);
        let mut dpr = device_dpr;
        if rect.width() > 0.0 && rect.height() > 0.0 {
            let ratio_w = width as f64 / rect.width();
            let ratio_h = height as f64 / rect.height();
            let ratio = ratio_w.min(ratio_h);
            if ratio < device_dpr - 0.01 {
                dpr = ratio;
            }
        }

        // Request device
        let (device, queue): (wgpu::Device, wgpu::Queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("noirtty-device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_webgl2_defaults(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
            })
            .await
            .map_err(|e| JsValue::from_str(&format!("Failed to create device: {}", e)))?;

        // Configure surface
        let surface_caps = surface.get_capabilities(&adapter);
        let force_rgba = should_force_rgba();
        if force_rgba {
            tracing::warn!("WebGPU: forcing RGBA surface format via ?force_rgba=1");
        }
        let surface_format = if force_rgba {
            surface_caps
                .formats
                .iter()
                .find(|f| **f == wgpu::TextureFormat::Rgba8Unorm)
                .copied()
                .or_else(|| {
                    surface_caps
                        .formats
                        .iter()
                        .find(|f| **f == wgpu::TextureFormat::Rgba8UnormSrgb)
                        .copied()
                })
                .unwrap_or(surface_caps.formats[0])
        } else if is_ios_safari() {
            surface_caps
                .formats
                .iter()
                .find(|f| **f == wgpu::TextureFormat::Bgra8UnormSrgb)
                .copied()
                .or_else(|| {
                    surface_caps
                        .formats
                        .iter()
                        .find(|f| **f == wgpu::TextureFormat::Bgra8Unorm)
                        .copied()
                })
                .or_else(|| surface_caps.formats.iter().find(|f| f.is_srgb()).copied())
                .unwrap_or(surface_caps.formats[0])
        } else {
            surface_caps
                .formats
                .iter()
                .find(|f| f.is_srgb())
                .copied()
                .unwrap_or(surface_caps.formats[0])
        };
        let alpha_mode = surface_caps
            .alpha_modes
            .iter()
            .copied()
            .find(|m| *m == wgpu::CompositeAlphaMode::Opaque)
            .unwrap_or(surface_caps.alpha_modes[0]);

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width,
            height,
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        // Initialize glyphon for text rendering
        let mut font_system = create_font_system();
        let swash_cache = SwashCache::new();
        let cache = Cache::new(&device);
        let color_mode = if is_apple_safari() {
            ColorMode::Web
        } else {
            ColorMode::Accurate
        };
        let mut text_atlas = TextAtlas::with_color_mode(&device, &queue, &cache, surface_format, color_mode);
        let text_renderer = TextRenderer::new(
            &mut text_atlas,
            &device,
            wgpu::MultisampleState::default(),
            None,
        );

        // Create viewport
        let viewport = Viewport::new(&device, &cache);

        // Create text buffer for terminal content
        let font_size = 14.0_f64;
        let line_height = font_size * 1.2;
        let mut text_buffer =
            Buffer::new(&mut font_system, Metrics::new(font_size as f32, line_height as f32));
        text_buffer.set_size(&mut font_system, Some(logical_width), Some(logical_height));
        text_buffer.set_wrap(&mut font_system, Wrap::None);

        // Measure cell dimensions using a sample character
        let cell_width = measure_char_width(
            &mut font_system,
            &mut text_buffer,
            font_size,
            FontFamily::Monospace.as_family(),
        );
        let cell_height = line_height;

        // Calculate initial grid size
        let cols = ((width as f64 / dpr) / cell_width).floor() as u16;
        let rows = ((height as f64 / dpr) / cell_height).floor() as u16;

        // Create background pipeline
        let background_pipeline = BackgroundPipeline::new(&device, surface_format);

        // Create uniform buffer
        let uniforms = GridUniforms {
            canvas_size: [width as f32, height as f32],
            cell_size: [(cell_width * dpr) as f32, (cell_height * dpr) as f32],
            grid_size: [cols as f32, rows as f32],
            _padding: [0.0, 0.0],
            selection_color: [0.15, 0.31, 0.47, 1.0], // #264f78
            cursor_color: [0.75, 0.75, 0.75, 1.0],    // #c0c0c0
            background_color: [0.118, 0.118, 0.118, 1.0], // #1e1e1e
        };

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniform-buffer"),
            size: std::mem::size_of::<GridUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        // Create uniform bind group
        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("uniform-bind-group"),
            layout: background_pipeline.bind_group_layout(),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        // Create instance buffer (start with capacity for 80x24 terminal)
        let instance_capacity = (cols as usize * rows as usize).max(80 * 24);
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instance-buffer"),
            size: (instance_capacity * std::mem::size_of::<CellInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(WebGpuRenderer {
            device,
            queue,
            surface,
            surface_config,
            canvas,
            background_pipeline,
            instance_buffer,
            instance_capacity,
            uniform_buffer,
            uniform_bind_group,
            font_system,
            swash_cache,
            text_atlas,
            text_renderer,
            text_buffer,
            viewport,
            cols,
            rows,
            cell_width,
            cell_height,
            font_size,
            dpr,
            background_color: [0.118, 0.118, 0.118, 1.0],
            selection_color: [0.15, 0.31, 0.47, 1.0],
            cursor_color: [0.75, 0.75, 0.75, 1.0],
            cursor_text_color: [0.118, 0.118, 0.118, 1.0],
            cursor_text_color_u8: [30, 30, 30],
            default_fg: [229, 229, 229],
            font_family: FontFamily::Monospace,
            frame_counter: 0,
            last_text_runs: 0,
            debug_text: false,
            max_surface_dim,
        })
    }

    /// Resize the renderer
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), JsValue> {
        self.cols = cols;
        self.rows = rows;

        // Ensure instance buffer has enough capacity
        let required_capacity = cols as usize * rows as usize;
        if required_capacity > self.instance_capacity {
            self.instance_capacity = required_capacity;
            self.instance_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("instance-buffer"),
                size: (self.instance_capacity * std::mem::size_of::<CellInstance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }

        self.update_uniforms();
        Ok(())
    }

    /// Update canvas dimensions from window resize
    pub fn set_size(&mut self, width: u32, height: u32) -> Result<(), JsValue> {
        let width = width.min(self.max_surface_dim).max(1);
        let height = height.min(self.max_surface_dim).max(1);
        self.canvas.set_width(width);
        self.canvas.set_height(height);

        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);

        // Update text buffer size
        if let Some(window) = web_sys::window() {
            let device_dpr = window.device_pixel_ratio();
            let rect = self.canvas.get_bounding_client_rect();
            if rect.width() > 0.0 && rect.height() > 0.0 {
                let ratio_w = width as f64 / rect.width();
                let ratio_h = height as f64 / rect.height();
                let ratio = ratio_w.min(ratio_h);
                self.dpr = if ratio < device_dpr - 0.01 { ratio } else { device_dpr };
            } else {
                self.dpr = device_dpr;
            }
        }
        let logical_width = width as f32 / self.dpr as f32;
        let logical_height = height as f32 / self.dpr as f32;
        self.text_buffer
            .set_size(&mut self.font_system, Some(logical_width), Some(logical_height));

        // Update viewport
        self.viewport.update(
            &self.queue,
            glyphon::Resolution {
                width,
                height,
            },
        );

        self.update_uniforms();
        Ok(())
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
        self.font_size = font_size;
        let line_height = (font_size * 1.2) as f32;

        // Update text buffer metrics
        self.text_buffer
            .set_metrics(&mut self.font_system, Metrics::new(font_size as f32, line_height));

        // Measure new cell dimensions
        self.font_family = parse_font_family(font_stack);
        if !font_family_exists(&self.font_system, &self.font_family) {
            self.font_family = FontFamily::Monospace;
        }
        self.cell_width = measure_char_width(
            &mut self.font_system,
            &mut self.text_buffer,
            font_size,
            self.font_family.as_family(),
        );
        self.cell_height = font_size * 1.2;

        self.background_color = parse_color(background);
        self.selection_color = parse_color(selection);
        self.cursor_color = parse_color(cursor);
        self.cursor_text_color = parse_color(cursor_text);
        self.cursor_text_color_u8 = color_f32_to_u8(self.cursor_text_color);

        self.update_uniforms();
        Ok(())
    }

    /// Calculate columns and rows that fit in the given physical dimensions
    pub fn calculate_grid_size(&self, width: u32, height: u32) -> (u16, u16) {
        let logical_width = width as f64 / self.dpr;
        let logical_height = height as f64 / self.dpr;

        let cols = (logical_width / self.cell_width).floor() as u16;
        let rows = (logical_height / self.cell_height).floor() as u16;

        (cols.max(1), rows.max(1))
    }

    /// Convert pixel coordinates to cell coordinates
    pub fn pixel_to_cell(&self, x: u32, y: u32) -> (u16, u16) {
        let logical_x = x as f64 / self.dpr;
        let logical_y = y as f64 / self.dpr;

        let col = (logical_x / self.cell_width).floor() as u16;
        let row = (logical_y / self.cell_height).floor() as u16;

        (
            col.min(self.cols.saturating_sub(1)),
            row.min(self.rows.saturating_sub(1)),
        )
    }

    /// Render the terminal
    pub fn render(&mut self, terminal: &Terminal) -> Result<(), JsValue> {
        self.frame_counter = self.frame_counter.wrapping_add(1);
        // Get surface texture
        let output = self
            .surface
            .get_current_texture()
            .map_err(|e| JsValue::from_str(&format!("Failed to get surface texture: {}", e)))?;

        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        // Build instance data for backgrounds
        let instances = self.build_instances(terminal);

        // Upload instance data
        if !instances.is_empty() {
            self.queue.write_buffer(
                &self.instance_buffer,
                0,
                bytemuck::cast_slice(&instances),
            );
        }

        // Update viewport
        self.viewport.update(
            &self.queue,
            glyphon::Resolution {
                width: self.surface_config.width,
                height: self.surface_config.height,
            },
        );

        // Build text content and update buffer
        if self.debug_text {
            self.update_text_buffer_debug();
        } else {
            self.update_text_buffer(terminal);
        }

        // Create text area and prepare renderer
        let text_area = TextArea {
            buffer: &self.text_buffer,
            left: 0.0,
            top: 0.0,
            scale: self.dpr as f32,
            bounds: TextBounds::default(),
            default_color: Color::rgb(self.default_fg[0], self.default_fg[1], self.default_fg[2]),
            custom_glyphs: &[],
        };

        // Prepare text renderer
        self.text_renderer
            .prepare(
                &self.device,
                &self.queue,
                &mut self.font_system,
                &mut self.text_atlas,
                &self.viewport,
                [text_area],
                &mut self.swash_cache,
            )
            .map_err(|e| JsValue::from_str(&format!("Text prepare failed: {:?}", e)))?;

        // Create command encoder
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("render-encoder"),
            });

        // Render backgrounds (pass 1)
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("background-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: self.background_color[0] as f64,
                            g: self.background_color[1] as f64,
                            b: self.background_color[2] as f64,
                            a: self.background_color[3] as f64,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            if !instances.is_empty() {
                render_pass.set_pipeline(self.background_pipeline.pipeline());
                render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                render_pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
                render_pass.draw(0..6, 0..instances.len() as u32);
            }
        }

        // Render text (pass 2) to avoid Safari pipeline issues
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("text-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            self.text_renderer
                .render(&self.text_atlas, &self.viewport, &mut render_pass)
                .map_err(|e| JsValue::from_str(&format!("Text render failed: {:?}", e)))?;
        }

        // Submit
        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        // Trim atlas to free unused glyphs
        if self.frame_counter % 120 == 0 {
            self.text_atlas.trim();
        }

        Ok(())
    }

    pub fn debug_text_runs(&self) -> u32 {
        self.last_text_runs
    }

    pub fn set_debug_text(&mut self, enabled: bool) {
        self.debug_text = enabled;
    }

    fn update_uniforms(&self) {
        let uniforms = GridUniforms {
            canvas_size: [
                self.surface_config.width as f32,
                self.surface_config.height as f32,
            ],
            cell_size: [
                (self.cell_width * self.dpr) as f32,
                (self.cell_height * self.dpr) as f32,
            ],
            grid_size: [self.cols as f32, self.rows as f32],
            _padding: [0.0, 0.0],
            selection_color: self.selection_color,
            cursor_color: self.cursor_color,
            background_color: self.background_color,
        };
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
    }

    fn build_instances(&self, terminal: &Terminal) -> Vec<CellInstance> {
        let (cursor_col, cursor_row) = terminal.cursor_position();
        let cursor_visible = terminal.cursor_visible();
        let selection = terminal.selection_range();
        let default_bg = [30, 30, 30];

        let mut instances = Vec::with_capacity(self.cols as usize * self.rows as usize);

        for (col, row, cell) in terminal.iter_cells() {
            // Check if cell is selected
            let is_selected = if let Some((start, end)) = selection {
                let pos = (row, col);
                pos >= start && pos <= end
            } else {
                false
            };

            // Check if cursor
            let is_cursor = cursor_visible && col == cursor_col && row == cursor_row;

            // Determine if we need to render this cell's background
            let has_bg = is_selected || is_cursor || cell.bg != default_bg;

            // Compute background color
            let bg_color = if is_cursor {
                self.cursor_color
            } else if is_selected {
                self.selection_color
            } else {
                [
                    cell.bg[0] as f32 / 255.0,
                    cell.bg[1] as f32 / 255.0,
                    cell.bg[2] as f32 / 255.0,
                    1.0,
                ]
            };

            // Flags: bit 0 = has_bg, bit 1 = is_cursor, bit 2 = is_selected, bit 3 = underline
            let flags = (has_bg as u32)
                | ((is_cursor as u32) << 1)
                | ((is_selected as u32) << 2)
                | ((cell.underline as u32) << 3);

            let fg_color = [
                cell.fg[0] as f32 / 255.0,
                cell.fg[1] as f32 / 255.0,
                cell.fg[2] as f32 / 255.0,
                1.0,
            ];

            instances.push(CellInstance {
                pos: [col as f32, row as f32],
                bg_color,
                flags,
                fg_color,
            });
        }

        instances
    }

    fn update_text_buffer(&mut self, terminal: &Terminal) {
        let (cursor_col, cursor_row) = terminal.cursor_position();
        let cursor_visible = terminal.cursor_visible();
        let base_attrs = Attrs::new().family(self.font_family.as_family());
        let mut spans: Vec<(String, Option<[u8; 3]>)> = Vec::new();
        let mut current_color: Option<[u8; 3]> = None;
        let mut current_segment = String::new();

        for row in 0..self.rows {
            for col in 0..self.cols {
                let cell = terminal
                    .cell(col, row)
                    .map(|cell| (cell.c, cell.fg))
                    .unwrap_or((' ', self.default_fg));
                let is_cursor = cursor_visible && col == cursor_col && row == cursor_row;
                let fg = if is_cursor {
                    self.cursor_text_color_u8
                } else {
                    cell.1
                };
                let ch = if cell.0 > ' ' { cell.0 } else { ' ' };

                if current_color != Some(fg) {
                    push_span(&mut spans, &mut current_segment, current_color);
                    current_color = Some(fg);
                }
                current_segment.push(ch);
            }

            current_segment.push('\n');
            push_span(&mut spans, &mut current_segment, current_color);
            current_color = None;
        }

        self.text_buffer.set_rich_text(
            &mut self.font_system,
            spans.iter().map(|(s, color)| {
                let attrs = match color {
                    Some(c) => base_attrs.clone().color(Color::rgb(c[0], c[1], c[2])),
                    None => base_attrs.clone(),
                };
                (s.as_str(), attrs)
            }),
            &base_attrs,
            Shaping::Advanced,
            None,
        );

        self.last_text_runs = self.text_buffer.layout_runs().count() as u32;
    }

    fn update_text_buffer_debug(&mut self) {
        let attrs = Attrs::new()
            .family(self.font_family.as_family())
            .color(Color::rgb(255, 255, 255));
        self.text_buffer.set_text(
            &mut self.font_system,
            "HELLO WEBGPU\nIt works if you see this.",
            &attrs,
            Shaping::Advanced,
            None,
        );
        self.last_text_runs = self.text_buffer.layout_runs().count() as u32;
    }

    /// Maximum surface dimension supported by the active backend.
    pub fn max_surface_dim(&self) -> u32 {
        self.max_surface_dim
    }
}

/// Measure the width of a character using the font system
fn measure_char_width(
    font_system: &mut FontSystem,
    buffer: &mut Buffer,
    font_size: f64,
    family: Family<'_>,
) -> f64 {
    // Set a test character to measure
    let attrs = Attrs::new().family(family);
    buffer.set_text(
        font_system,
        "M",
        &attrs,
        Shaping::Advanced,
        None,
    );

    // Get the layout and measure
    buffer.shape_until_scroll(font_system, false);

    // For monospace fonts, width should be consistent
    // Use an approximate ratio if measurement fails
    if let Some(line) = buffer.lines.first() {
        if let Some(layout) = line.layout_opt() {
            if let Some(layout_line) = layout.first() {
                if !layout_line.glyphs.is_empty() {
                    return layout_line.glyphs[0].w as f64;
                }
            }
        }
    }

    // Fallback: approximate monospace width
    font_size * 0.6
}

/// Parse a CSS color string (e.g., "#1e1e1e") to RGBA floats
fn parse_color(color: &str) -> [f32; 4] {
    if color.starts_with('#') && color.len() == 7 {
        let r = u8::from_str_radix(&color[1..3], 16).unwrap_or(0);
        let g = u8::from_str_radix(&color[3..5], 16).unwrap_or(0);
        let b = u8::from_str_radix(&color[5..7], 16).unwrap_or(0);
        [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]
    } else {
        [0.0, 0.0, 0.0, 1.0]
    }
}

fn push_span(
    spans: &mut Vec<(String, Option<[u8; 3]>)>,
    segment: &mut String,
    color: Option<[u8; 3]>,
) {
    if segment.is_empty() {
        return;
    }
    spans.push((std::mem::take(segment), color));
}

fn color_f32_to_u8(color: [f32; 4]) -> [u8; 3] {
    [
        (color[0].clamp(0.0, 1.0) * 255.0).round() as u8,
        (color[1].clamp(0.0, 1.0) * 255.0).round() as u8,
        (color[2].clamp(0.0, 1.0) * 255.0).round() as u8,
    ]
}

fn create_font_system() -> FontSystem {
    let data_nerd = include_bytes!("../../../assets/fonts/0xProtoNerdFontMono-Regular.ttf");
    let data_courier = include_bytes!("../../../assets/fonts/courier_new.ttf");
    let source_nerd = glyphon::fontdb::Source::Binary(Arc::new(data_nerd.to_vec()));
    let source_courier = glyphon::fontdb::Source::Binary(Arc::new(data_courier.to_vec()));
    let mut font_system = FontSystem::new_with_fonts([source_nerd, source_courier]);
    font_system
        .db_mut()
        .set_monospace_family("0xProto Nerd Font Mono");
    font_system
}

#[derive(Clone, Debug)]
enum FontFamily {
    Monospace,
    Serif,
    SansSerif,
    Cursive,
    Fantasy,
    Name(String),
}

impl FontFamily {
    fn as_family(&self) -> Family<'_> {
        match self {
            FontFamily::Monospace => Family::Monospace,
            FontFamily::Serif => Family::Serif,
            FontFamily::SansSerif => Family::SansSerif,
            FontFamily::Cursive => Family::Cursive,
            FontFamily::Fantasy => Family::Fantasy,
            FontFamily::Name(name) => Family::Name(name.as_str()),
        }
    }
}

fn parse_font_family(font_stack: &str) -> FontFamily {
    for part in font_stack.split(',') {
        let name = part.trim().trim_matches('"').trim_matches('\'').trim();
        if name.is_empty() {
            continue;
        }

        let lower = name.to_ascii_lowercase();
        return match lower.as_str() {
            "monospace" => FontFamily::Monospace,
            "serif" => FontFamily::Serif,
            "sans-serif" => FontFamily::SansSerif,
            "cursive" => FontFamily::Cursive,
            "fantasy" => FontFamily::Fantasy,
            _ => FontFamily::Name(name.to_string()),
        };
    }

    FontFamily::Monospace
}

fn font_family_exists(font_system: &FontSystem, family: &FontFamily) -> bool {
    let name = match family {
        FontFamily::Name(name) => name.as_str(),
        _ => return true,
    };

    font_system.db().faces().any(|face| {
        face.families
            .iter()
            .any(|(fam, _)| fam.eq_ignore_ascii_case(name))
    })
}

fn is_apple_safari() -> bool {
    let Some(window) = web_sys::window() else { return false };
    let Ok(ua) = window.navigator().user_agent() else { return false };
    let ua = ua.to_ascii_lowercase();
    let is_safari = ua.contains("safari")
        && !ua.contains("chrome")
        && !ua.contains("crios")
        && !ua.contains("fxios")
        && !ua.contains("edgios");
    let is_apple = ua.contains("mac os x") || ua.contains("iphone") || ua.contains("ipad") || ua.contains("ipod");
    is_safari && is_apple
}

fn is_ios_safari() -> bool {
    let Some(window) = web_sys::window() else { return false };
    let Ok(ua) = window.navigator().user_agent() else { return false };
    let ua = ua.to_ascii_lowercase();
    let is_safari = ua.contains("safari")
        && !ua.contains("chrome")
        && !ua.contains("crios")
        && !ua.contains("fxios")
        && !ua.contains("edgios");
    let is_ios = ua.contains("iphone") || ua.contains("ipad") || ua.contains("ipod");
    is_safari && is_ios
}

fn should_force_rgba() -> bool {
    let Some(window) = web_sys::window() else { return false };
    let Ok(location) = window.location().search() else { return false };
    let search = location.to_ascii_lowercase();
    search.contains("force_rgba=1") || search.contains("force_rgba=true")
}

fn should_force_gl() -> bool {
    let Some(window) = web_sys::window() else { return false };
    let Ok(location) = window.location().search() else { return false };
    let search = location.to_ascii_lowercase();
    search.contains("force_gl=1") || search.contains("force_gl=true")
}

fn should_force_webgpu() -> bool {
    let Some(window) = web_sys::window() else { return false };
    let Ok(location) = window.location().search() else { return false };
    let search = location.to_ascii_lowercase();
    search.contains("force_webgpu=1") || search.contains("force_webgpu=true")
}

fn browser_has_webgpu() -> bool {
    let Some(window) = web_sys::window() else { return false };
    let navigator = window.navigator();
    match Reflect::get(&navigator, &JsValue::from_str("gpu")) {
        Ok(val) => !val.is_undefined() && !val.is_null(),
        Err(_) => false,
    }
}

async fn create_surface_and_adapter(
    canvas: &HtmlCanvasElement,
    backends: wgpu::Backends,
) -> Result<(wgpu::Surface<'static>, wgpu::Adapter), JsValue> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends,
        ..Default::default()
    });
    let surface = instance
        .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone().into()))
        .map_err(|e| JsValue::from_str(&format!("Failed to create surface: {}", e)))?;
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        })
        .await
        .map_err(|e| JsValue::from_str(&format!("No suitable GPU adapter: {}", e)))?;
    Ok((surface, adapter))
}

fn log_adapter_info(adapter: &wgpu::Adapter, surface: &wgpu::Surface) {
    let limits = adapter.limits();
    let features = adapter.features();
    let caps = surface.get_capabilities(adapter);
    tracing::info!("WebGPU limits: max_texture_dimension_2d={}", limits.max_texture_dimension_2d);
    tracing::info!("WebGPU limits: max_texture_array_layers={}", limits.max_texture_array_layers);
    tracing::info!("WebGPU limits: max_bind_groups={}", limits.max_bind_groups);
    tracing::info!("WebGPU features: {:?}", features);
    tracing::info!("WebGPU surface formats: {:?}", caps.formats);
    tracing::info!("WebGPU alpha modes: {:?}", caps.alpha_modes);
    tracing::info!("WebGPU present modes: {:?}", caps.present_modes);
}
