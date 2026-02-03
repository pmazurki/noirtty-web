//! Canvas 2D terminal renderer (fallback)

use crate::terminal::Terminal;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

/// Canvas 2D renderer for terminal
pub struct Canvas2DRenderer {
    canvas: HtmlCanvasElement,
    ctx: CanvasRenderingContext2d,
    width: u32,
    height: u32,
    cols: u16,
    rows: u16,
    cell_width: f64,
    cell_height: f64,
    font: String,
    dpr: f64,
    background: String,
    selection: String,
    cursor: String,
    cursor_text: String,
}

impl Canvas2DRenderer {
    /// Create a new Canvas 2D renderer
    pub async fn new(canvas_id: &str) -> Result<Self, JsValue> {
        let window = web_sys::window().ok_or("No window")?;
        let document = window.document().ok_or("No document")?;
        let canvas: HtmlCanvasElement = document
            .get_element_by_id(canvas_id)
            .ok_or("Canvas not found")?
            .dyn_into()?;

        let dpr = window.device_pixel_ratio();
        let rect = canvas.get_bounding_client_rect();
        let width = (rect.width() * dpr) as u32;
        let height = (rect.height() * dpr) as u32;

        // Set canvas physical size
        canvas.set_width(width);
        canvas.set_height(height);

        // Get context
        let ctx: CanvasRenderingContext2d = canvas
            .get_context("2d")?
            .ok_or("Failed to get 2d context")?
            .dyn_into()?;

        // Scale context for HiDPI
        ctx.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0)?;
        ctx.scale(dpr, dpr)?;

        // Use a proper monospace font stack including Nerd Fonts and system fonts
        let font_size = 14.0; // Logical pixels
        let font = format!(
            "{}px 'JetBrains Mono', 'Fira Code', 'MesloLGS NF', 'SF Mono', 'Monaco', 'Menlo', 'Consolas', 'Ubuntu Mono', 'Liberation Mono', 'DejaVu Sans Mono', 'Apple Color Emoji', 'Segoe UI Emoji', 'Segoe UI Symbol', 'Noto Color Emoji', 'Twemoji Mozilla', monospace",
            font_size
        );

        ctx.set_font(&font);
        ctx.set_text_baseline("top");

        // Measure actual character width
        let metrics = ctx.measure_text("M")?;
        let cell_width = metrics.width();
        let cell_height = font_size * 1.2;

        Ok(Canvas2DRenderer {
            canvas,
            ctx,
            width,
            height,
            cols: 80,
            rows: 24,
            cell_width,
            cell_height,
            font,
            dpr,
            background: "#1e1e1e".to_string(),
            selection: "#264f78".to_string(),
            cursor: "#c0c0c0".to_string(),
            cursor_text: "#1e1e1e".to_string(),
        })
    }

    /// Resize the renderer
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), JsValue> {
        self.cols = cols;
        self.rows = rows;
        self.ctx.set_font(&self.font);
        self.ctx.set_text_baseline("top");
        Ok(())
    }

    /// Update canvas dimensions from window resize
    pub fn set_size(&mut self, width: u32, height: u32) -> Result<(), JsValue> {
        // width/height passed here are physical pixels.
        self.width = width;
        self.height = height;

        // Update canvas physical size; this resets the context transform.
        self.canvas.set_width(width);
        self.canvas.set_height(height);

        // Reset transform and scale for HiDPI.
        self.ctx.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0)?;
        self.ctx.scale(self.dpr, self.dpr)?;
        self.ctx.set_font(&self.font);
        self.ctx.set_text_baseline("top");
        Ok(())
    }

    pub fn set_render_config(
        &mut self,
        font_size: f64,
        font_stack: &str,
        background: &str,
        selection: &str,
        cursor: &str,
        cursor_text: &str,
    ) -> Result<(), JsValue> {
        self.font = format!("{}px {}", font_size, font_stack);
        self.ctx.set_font(&self.font);
        self.ctx.set_text_baseline("top");

        let metrics = self.ctx.measure_text("M")?;
        self.cell_width = metrics.width();
        self.cell_height = font_size * 1.2;

        self.background = background.to_string();
        self.selection = selection.to_string();
        self.cursor = cursor.to_string();
        self.cursor_text = cursor_text.to_string();
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

    /// Maximum surface dimension for Canvas2D (no hard limit).
    pub fn max_surface_dim(&self) -> u32 {
        u32::MAX
    }

    /// Convert pixel coordinates to cell coordinates
    pub fn pixel_to_cell(&self, x: u32, y: u32) -> (u16, u16) {
        let logical_x = x as f64 / self.dpr;
        let logical_y = y as f64 / self.dpr;

        let col = (logical_x / self.cell_width).floor() as u16;
        let row = (logical_y / self.cell_height).floor() as u16;

        (col.min(self.cols.saturating_sub(1)), row.min(self.rows.saturating_sub(1)))
    }

    /// Render the terminal
    pub fn render(&mut self, terminal: &Terminal) -> Result<(), JsValue> {
        // Clear (using logical dimensions)
        self.ctx.set_fill_style_str(&self.background);
        let logical_width = self.width as f64 / self.dpr;
        let logical_height = self.height as f64 / self.dpr;
        self.ctx.fill_rect(0.0, 0.0, logical_width, logical_height);

        let (cursor_col, cursor_row) = terminal.cursor_position();
        let cursor_visible = terminal.cursor_visible();

        // Get selection range
        let selection = terminal.selection_range();

        self.ctx.set_font(&self.font);

        // Render cells
        for (col, row, cell) in terminal.iter_cells() {
            let x = col as f64 * self.cell_width;
            let y = row as f64 * self.cell_height;

            // Check if cell is selected
            let is_selected = if let Some((start, end)) = selection {
                let pos = (row, col);
                pos >= start && pos <= end
            } else {
                false
            };

            // Background
            if is_selected {
                // Selection color (e.g., light blue/gray)
                self.ctx.set_fill_style_str(&self.selection);
                self.ctx.fill_rect(x, y, self.cell_width + 1.0, self.cell_height);
            } else if cell.bg != [30, 30, 30] {
                self.ctx.set_fill_style_str(&format!(
                    "rgb({},{},{})", cell.bg[0], cell.bg[1], cell.bg[2]
                ));
                self.ctx.fill_rect(x, y, self.cell_width + 1.0, self.cell_height);
            }

            // Cursor block
            if cursor_visible && col == cursor_col && row == cursor_row {
                self.ctx.set_fill_style_str(&self.cursor);
                self.ctx.fill_rect(x, y, self.cell_width, self.cell_height);
                self.ctx.set_fill_style_str(&self.cursor_text); // Text color in cursor
            } else {
                self.ctx.set_fill_style_str(&format!(
                    "rgb({},{},{})", cell.fg[0], cell.fg[1], cell.fg[2]
                ));
            }

            // Character
            if cell.c > ' ' {
                self.ctx.fill_text(&cell.c.to_string(), x, y + 2.0)?;
            }

            // Underline
            if cell.underline {
                self.ctx.set_stroke_style_str(&format!(
                    "rgb({},{},{})", cell.fg[0], cell.fg[1], cell.fg[2]
                ));
                self.ctx.begin_path();
                self.ctx.move_to(x, y + self.cell_height - 2.0);
                self.ctx.line_to(x + self.cell_width, y + self.cell_height - 2.0);
                self.ctx.stroke();
            }
        }

        Ok(())
    }
}
