//! NoirTTY Web Client - WebGPU/WebTransport Edition
//!
//! Modern terminal client using WebGPU for rendering and WebTransport for I/O.

use wasm_bindgen::prelude::*;

mod renderer;
mod terminal;
mod transport;
mod input;

pub use renderer::Renderer;
pub use terminal::Terminal;
pub use transport::Transport;
pub use input::InputHandler;

/// Initialize panic hook for better WASM debugging
#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
    tracing_wasm::set_as_global_default();
}

/// Main terminal application
#[wasm_bindgen]
pub struct NoirTTYWeb {
    terminal: Terminal,
    renderer: Option<Renderer>,
    transport: Option<Transport>,
    input: InputHandler,
    pending_render_config: Option<RenderConfig>,
    pending_max_frames: Option<usize>,
    pending_min_interval_ms: Option<u32>,
    frame_count: u64,
}

struct RenderConfig {
    font_size: f64,
    font_stack: String,
    background: String,
    selection: String,
    cursor: String,
    cursor_text: String,
}

#[wasm_bindgen]
impl NoirTTYWeb {
    /// Create a new NoirTTY terminal instance
    #[wasm_bindgen(constructor)]
    pub fn new(_canvas_id: &str) -> Result<NoirTTYWeb, JsValue> {
        let terminal = Terminal::new(80, 24);
        let input = InputHandler::new();

        Ok(NoirTTYWeb {
            terminal,
            renderer: None,
            transport: None,
            input,
            pending_render_config: None,
            pending_max_frames: None,
            pending_min_interval_ms: None,
            frame_count: 0,
        })
    }

    /// Initialize the WebGPU renderer
    #[wasm_bindgen]
    pub async fn init_renderer(&mut self, canvas_id: &str) -> Result<(), JsValue> {
        let renderer = Renderer::new(canvas_id).await?;
        let mut renderer = renderer;
        if let Some(config) = self.pending_render_config.take() {
            renderer.set_render_config(
                config.font_size,
                &config.font_stack,
                &config.background,
                &config.selection,
                &config.cursor,
                &config.cursor_text,
            )?;
        }
        self.renderer = Some(renderer);
        Ok(())
    }

    /// Configure renderer (font + colors)
    #[wasm_bindgen]
    pub fn set_render_config(
        &mut self,
        font_size: f64,
        font_stack: &str,
        background: &str,
        selection: &str,
        cursor: &str,
        cursor_text: &str,
    ) -> Result<(), JsValue> {
        if let Some(ref mut renderer) = self.renderer {
            renderer.set_render_config(
                font_size,
                font_stack,
                background,
                selection,
                cursor,
                cursor_text,
            )?;
        } else {
            self.pending_render_config = Some(RenderConfig {
                font_size,
                font_stack: font_stack.to_string(),
                background: background.to_string(),
                selection: selection.to_string(),
                cursor: cursor.to_string(),
                cursor_text: cursor_text.to_string(),
            });
        }

        Ok(())
    }

    /// Connect to WebTransport server
    #[wasm_bindgen]
    pub async fn connect(&mut self, url: &str) -> Result<(), JsValue> {
        let transport = Transport::connect(url).await?;
        if let Some(max_frames) = self.pending_max_frames.take() {
            transport.set_max_frames(max_frames);
        }
        if let Some(min_interval_ms) = self.pending_min_interval_ms.take() {
            transport.send_quality(min_interval_ms)?;
        }
        self.transport = Some(transport);
        Ok(())
    }

    /// Limit the number of frames kept in the client queue (0 = unlimited).
    #[wasm_bindgen]
    pub fn set_max_frames_in_queue(&mut self, max_frames: u32) {
        let max_frames = max_frames as usize;
        if let Some(ref transport) = self.transport {
            transport.set_max_frames(max_frames);
        } else {
            self.pending_max_frames = Some(max_frames);
        }
    }

    /// Throttle server frame rate (0 = no throttle).
    #[wasm_bindgen]
    pub fn set_frame_throttle_ms(&mut self, min_interval_ms: u32) -> Result<(), JsValue> {
        if let Some(ref transport) = self.transport {
            transport.send_quality(min_interval_ms)?;
        } else {
            self.pending_min_interval_ms = Some(min_interval_ms);
        }
        Ok(())
    }

    /// WebSocket connection state (0=connecting,1=open,2=closing,3=closed)
    #[wasm_bindgen]
    pub fn connection_state(&self) -> u16 {
        self.transport
            .as_ref()
            .map(|t| t.ready_state())
            .unwrap_or(3)
    }

    /// Number of frames queued in the client transport.
    #[wasm_bindgen]
    pub fn transport_queue_len(&self) -> u32 {
        self.transport
            .as_ref()
            .map(|t| t.queue_len() as u32)
            .unwrap_or(0)
    }

    /// Total bytes received by transport.
    #[wasm_bindgen]
    pub fn transport_bytes_received(&self) -> u64 {
        self.transport
            .as_ref()
            .map(|t| t.bytes_received())
            .unwrap_or(0)
    }

    /// Total messages received by transport.
    #[wasm_bindgen]
    pub fn transport_messages_received(&self) -> u64 {
        self.transport
            .as_ref()
            .map(|t| t.messages_received())
            .unwrap_or(0)
    }

    /// Reset transport counters.
    #[wasm_bindgen]
    pub fn transport_reset_counters(&self) {
        if let Some(ref transport) = self.transport {
            transport.reset_counters();
        }
    }

    /// Maximum surface dimension supported by the active renderer.
    #[wasm_bindgen]
    pub fn max_surface_dim(&self) -> u32 {
        self.renderer
            .as_ref()
            .map(|r| r.max_surface_dim())
            .unwrap_or(u32::MAX)
    }

    /// Send input to terminal
    #[wasm_bindgen]
    pub fn send_input(&mut self, data: &str) -> Result<(), JsValue> {
        if let Some(ref mut transport) = self.transport {
            transport.send(data.as_bytes())?;
        }
        Ok(())
    }

    /// Handle keyboard event
    #[wasm_bindgen]
    pub fn on_key(&mut self, code: &str, key: &str, ctrl: bool, alt: bool, meta: bool, shift: bool) -> Result<(), JsValue> {
        if let Some(data) = self.input.process_key(code, key, ctrl, alt, meta, shift) {
            // LOCAL ECHO: Try to predict simple printable characters
            // Only predict single-byte printable ASCII (no modifiers except shift)
            if data.len() == 1 && !ctrl && !alt && !meta {
                if let Some(c) = data.chars().next() {
                    // write_char_speculative returns true if it handled the char
                    // This provides instant visual feedback before server response
                    self.terminal.write_char_speculative(c);
                }
            }

            // Always send to server - server is authoritative
            if let Some(ref transport) = self.transport {
                transport.send(data.as_bytes())?;
            }
        }
        Ok(())
    }

    /// Resize terminal
    #[wasm_bindgen]
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), JsValue> {
        if cols == self.terminal.cols() && rows == self.terminal.rows() {
            return Ok(());
        }
        self.terminal.resize(cols, rows);

        if let Some(ref mut transport) = self.transport {
            transport.send_resize(cols, rows)?;
        }

        if let Some(ref mut renderer) = self.renderer {
            renderer.resize(cols, rows)?;
        }

        Ok(())
    }

    /// Scroll terminal viewport (positive = scroll up)
    #[wasm_bindgen]
    pub fn scroll(&mut self, delta: i32) -> Result<(), JsValue> {
        if let Some(ref transport) = self.transport {
            transport.send_scroll(delta)?;
        }
        Ok(())
    }

    /// Update size based on available dimensions (pixels)
    #[wasm_bindgen]
    pub fn update_size(&mut self, width: u32, height: u32) -> Result<(), JsValue> {
        let (cols, rows) = if let Some(ref mut renderer) = self.renderer {
            renderer.set_size(width, height)?;
            renderer.calculate_grid_size(width, height)
        } else {
            (80, 24)
        };
        
        self.resize(cols, rows)
    }

    /// Handle mouse down
    #[wasm_bindgen]
    pub fn on_mouse_down(&mut self, x: u32, y: u32) {
        if let Some(ref renderer) = self.renderer {
            let (col, row) = renderer.pixel_to_cell(x, y);
            self.terminal.start_selection(col, row);
        }
    }

    /// Handle mouse move
    #[wasm_bindgen]
    pub fn on_mouse_move(&mut self, x: u32, y: u32) {
        if let Some(ref renderer) = self.renderer {
            let (col, row) = renderer.pixel_to_cell(x, y);
            self.terminal.update_selection(col, row);
        }
    }

    /// Handle mouse up
    #[wasm_bindgen]
    pub fn on_mouse_up(&mut self) {
        self.terminal.end_selection();
    }

    /// Render frame - call from requestAnimationFrame
    #[wasm_bindgen]
    pub fn render(&mut self) -> Result<(), JsValue> {
        // Process incoming data from transport
        if let Some(ref mut transport) = self.transport {
            while let Some(frame) = transport.try_recv() {
                self.terminal.apply_frame(frame);
                self.frame_count = self.frame_count.wrapping_add(1);
            }
        }

        // Only render if terminal is dirty
        if self.terminal.is_dirty() {
            if let Some(ref mut renderer) = self.renderer {
                renderer.render(&self.terminal)?;
            }
            self.terminal.mark_clean();
        }

        Ok(())
    }

    /// Get current renderer type ("webgpu", "canvas2d", "uninitialized")
    #[wasm_bindgen]
    pub fn renderer_type(&self) -> String {
        match self.renderer.as_ref() {
            Some(renderer) => renderer.renderer_type().to_string(),
            None => "uninitialized".to_string(),
        }
    }

    /// Number of frames received from the server.
    #[wasm_bindgen]
    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    /// Debug: number of text layout runs in the renderer.
    #[wasm_bindgen]
    pub fn debug_text_runs(&self) -> u32 {
        match self.renderer.as_ref() {
            Some(renderer) => renderer.debug_text_runs(),
            None => 0,
        }
    }

    /// Debug: render a fixed test string instead of terminal content.
    #[wasm_bindgen]
    pub fn set_debug_text(&mut self, enabled: bool) {
        if let Some(ref mut renderer) = self.renderer {
            renderer.set_debug_text(enabled);
        }
    }

    /// Debug: get text of a row for quick inspection.
    #[wasm_bindgen]
    pub fn debug_row(&self, row: u16) -> String {
        if row >= self.terminal.rows() {
            return String::new();
        }
        let mut out = String::with_capacity(self.terminal.cols() as usize);
        for col in 0..self.terminal.cols() {
            if let Some(cell) = self.terminal.cell(col, row) {
                out.push(cell.c);
            } else {
                out.push(' ');
            }
        }
        out
    }

    /// Debug: get a single cell with fg/bg info.
    #[wasm_bindgen]
    pub fn debug_cell(&self, col: u16, row: u16) -> String {
        if let Some(cell) = self.terminal.cell(col, row) {
            return format!(
                "{} fg=#{:02x}{:02x}{:02x} bg=#{:02x}{:02x}{:02x}",
                cell.c,
                cell.fg[0],
                cell.fg[1],
                cell.fg[2],
                cell.bg[0],
                cell.bg[1],
                cell.bg[2]
            );
        }
        String::new()
    }

    /// Get terminal cols
    #[wasm_bindgen]
    pub fn cols(&self) -> u16 {
        self.terminal.cols()
    }

    /// Get terminal rows
    #[wasm_bindgen]
    pub fn rows(&self) -> u16 {
        self.terminal.rows()
    }

    /// Copy selection to clipboard
    #[wasm_bindgen]
    pub fn copy_selection(&self) -> Option<String> {
        self.terminal.get_selection()
    }

    /// Paste from clipboard
    #[wasm_bindgen]
    pub fn paste(&mut self, text: &str) -> Result<(), JsValue> {
        // Bracket paste mode for terminal safety
        let bracketed = format!("\x1b[200~{}\x1b[201~", text);
        self.send_input(&bracketed)
    }
}
