//! Terminal emulation using vte crate
//!
//! Provides VTE parsing and terminal grid state management.

use std::collections::VecDeque;
use serde::{Deserialize, Serialize};
use vte::{Params, Parser, Perform};

/// Terminal cell
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cell {
    pub c: char,
    pub fg: [u8; 3],
    pub bg: [u8; 3],
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Cell {
            c: ' ',
            fg: [229, 229, 229], // Default foreground (light gray)
            bg: [30, 30, 30],     // Default background (dark gray)
            bold: false,
            italic: false,
            underline: false,
            inverse: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TerminalFrame {
    pub cols: u16,
    pub rows: u16,
    pub cursor_col: u16,
    pub cursor_row: u16,
    pub cursor_visible: bool,
    pub cells: Vec<Cell>,
}

/// Cursor state
#[derive(Clone, Debug)]
pub struct Cursor {
    pub col: u16,
    pub row: u16,
    pub visible: bool,
}

impl Default for Cursor {
    fn default() -> Self {
        Cursor {
            col: 0,
            row: 0,
            visible: true,
        }
    }
}

/// Terminal grid and state
pub struct Terminal {
    cols: u16,
    rows: u16,
    grid: Vec<Cell>,
    cursor: Cursor,
    saved_cursor: Cursor,
    parser: Option<Parser>,
    dirty: bool,

    // Current text attributes
    current_fg: [u8; 3],
    current_bg: [u8; 3],
    current_bold: bool,
    current_italic: bool,
    current_underline: bool,
    current_inverse: bool,

    // Scrollback
    scrollback: Vec<Vec<Cell>>,
    max_scrollback: usize,

    // Modes
    _application_cursor_keys: bool,
    _bracketed_paste: bool,

    // Selection
    selection_start: Option<(u16, u16)>, // (row, col)
    selection_end: Option<(u16, u16)>,   // (row, col)
    selecting: bool,

    // Pending responses to send back to PTY (e.g., DSR)
    responses: VecDeque<Vec<u8>>,
}

impl Terminal {
    /// Create a new terminal with the given dimensions
    pub fn new(cols: u16, rows: u16) -> Self {
        let size = (cols as usize) * (rows as usize);
        let grid = vec![Cell::default(); size];

        Terminal {
            cols,
            rows,
            grid,
            cursor: Cursor::default(),
            saved_cursor: Cursor::default(),
            parser: Some(Parser::new()),
            dirty: true,
            current_fg: [229, 229, 229],
            current_bg: [30, 30, 30],
            current_bold: false,
            current_italic: false,
            current_underline: false,
            current_inverse: false,
            scrollback: Vec::new(),
            max_scrollback: 10000,
            _application_cursor_keys: false,
            _bracketed_paste: false,
            selection_start: None,
            selection_end: None,
            selecting: false,
            responses: VecDeque::new(),
        }
    }

    /// Process incoming bytes from PTY
    pub fn process(&mut self, data: &[u8]) {
        // Take parser out to avoid borrow conflict
        if let Some(mut parser) = self.parser.take() {
            parser.advance(self, data);
            self.parser = Some(parser);
        }
        self.dirty = true;
    }

    /// Replace the terminal state with a server-provided frame.
    pub fn apply_frame(&mut self, frame: TerminalFrame) {
        let cols = frame.cols.max(1);
        let rows = frame.rows.max(1);
        let size_changed = cols != self.cols || rows != self.rows;

        self.cols = cols;
        self.rows = rows;

        let expected = (cols as usize) * (rows as usize);
        if frame.cells.len() == expected {
            self.grid = frame.cells;
        } else {
            let mut grid = vec![Cell::default(); expected];
            for (dst, src) in grid.iter_mut().zip(frame.cells.into_iter()) {
                *dst = src;
            }
            self.grid = grid;
        }

        self.cursor.col = frame.cursor_col.min(cols.saturating_sub(1));
        self.cursor.row = frame.cursor_row.min(rows.saturating_sub(1));
        self.cursor.visible = frame.cursor_visible;

        if size_changed {
            self.clear_selection();
        }

        self.dirty = true;
    }

    /// Resize terminal
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let new_size = (cols as usize) * (rows as usize);
        let mut new_grid = vec![Cell::default(); new_size];

        // Copy existing content
        let min_cols = self.cols.min(cols) as usize;
        let min_rows = self.rows.min(rows) as usize;

        for row in 0..min_rows {
            for col in 0..min_cols {
                let old_idx = row * self.cols as usize + col;
                let new_idx = row * cols as usize + col;
                if old_idx < self.grid.len() && new_idx < new_grid.len() {
                    new_grid[new_idx] = self.grid[old_idx].clone();
                }
            }
        }

        self.cols = cols;
        self.rows = rows;
        self.grid = new_grid;

        // Clamp cursor
        self.cursor.col = self.cursor.col.min(cols.saturating_sub(1));
        self.cursor.row = self.cursor.row.min(rows.saturating_sub(1));
        self.dirty = true;
    }

    /// Get terminal columns
    pub fn cols(&self) -> u16 {
        self.cols
    }

    /// Get terminal rows
    pub fn rows(&self) -> u16 {
        self.rows
    }

    /// Check if terminal needs redraw
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Mark as clean after render
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    /// Get cell at position
    pub fn cell(&self, col: u16, row: u16) -> Option<&Cell> {
        if col < self.cols && row < self.rows {
            let idx = row as usize * self.cols as usize + col as usize;
            self.grid.get(idx)
        } else {
            None
        }
    }

    /// Get mutable cell at position
    fn cell_mut(&mut self, col: u16, row: u16) -> Option<&mut Cell> {
        if col < self.cols && row < self.rows {
            let idx = row as usize * self.cols as usize + col as usize;
            self.grid.get_mut(idx)
        } else {
            None
        }
    }

    /// Get cursor position
    pub fn cursor_position(&self) -> (u16, u16) {
        (self.cursor.col, self.cursor.row)
    }

    /// Is cursor visible
    pub fn cursor_visible(&self) -> bool {
        self.cursor.visible
    }

    /// Start selection at (col, row)
    pub fn start_selection(&mut self, col: u16, row: u16) {
        if col < self.cols && row < self.rows {
            self.selection_start = Some((row, col));
            self.selection_end = Some((row, col));
            self.selecting = true;
            self.dirty = true;
        }
    }

    /// Update selection to (col, row)
    pub fn update_selection(&mut self, col: u16, row: u16) {
        if self.selecting {
            let col = col.min(self.cols - 1);
            let row = row.min(self.rows - 1);
            if let Some(current_end) = self.selection_end {
                if current_end != (row, col) {
                    self.selection_end = Some((row, col));
                    self.dirty = true;
                }
            }
        }
    }

    /// End selection
    pub fn end_selection(&mut self) {
        self.selecting = false;
    }

    /// Clear selection
    pub fn clear_selection(&mut self) {
        if self.selection_start.is_some() {
            self.selection_start = None;
            self.selection_end = None;
            self.selecting = false;
            self.dirty = true;
        }
    }

    /// Get normalized selection range (start <= end)
    pub fn selection_range(&self) -> Option<((u16, u16), (u16, u16))> {
        match (self.selection_start, self.selection_end) {
            (Some(start), Some(end)) => {
                if start <= end {
                    Some((start, end))
                } else {
                    Some((end, start))
                }
            }
            _ => None,
        }
    }

    /// Get selection text
    pub fn get_selection(&self) -> Option<String> {
        let (start, end) = self.selection_range()?;
        let mut text = String::new();
        
        for row in start.0..=end.0 {
            let col_start = if row == start.0 { start.1 } else { 0 };
            let col_end = if row == end.0 { end.1 } else { self.cols - 1 };
            
            for col in col_start..=col_end {
                if let Some(cell) = self.cell(col, row) {
                    // Skip empty cells at the end of line unless it's part of a multi-line selection
                    // For simplicity, just add all chars for now
                    text.push(cell.c);
                }
            }
            
            if row < end.0 {
                text.push('\n');
            }
        }
        
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    /// Pop a pending response (if any).
    pub fn take_response(&mut self) -> Option<Vec<u8>> {
        self.responses.pop_front()
    }

    /// Iterate over all cells
    pub fn iter_cells(&self) -> impl Iterator<Item = (u16, u16, &Cell)> {
        (0..self.rows).flat_map(move |row| {
            (0..self.cols).map(move |col| {
                let idx = row as usize * self.cols as usize + col as usize;
                (col, row, &self.grid[idx])
            })
        })
    }

    /// Write character at current cursor position
    fn write_char(&mut self, c: char) {
        // Copy attributes before mutable borrow
        let fg = if self.current_inverse { self.current_bg } else { self.current_fg };
        let bg = if self.current_inverse { self.current_fg } else { self.current_bg };
        let bold = self.current_bold;
        let italic = self.current_italic;
        let underline = self.current_underline;
        let inverse = self.current_inverse;
        let col = self.cursor.col;
        let row = self.cursor.row;

        if let Some(cell) = self.cell_mut(col, row) {
            cell.c = c;
            cell.fg = fg;
            cell.bg = bg;
            cell.bold = bold;
            cell.italic = italic;
            cell.underline = underline;
            cell.inverse = inverse;
        }

        self.cursor.col += 1;
        if self.cursor.col >= self.cols {
            self.cursor.col = 0;
            self.cursor.row += 1;
            if self.cursor.row >= self.rows {
                self.scroll_up();
                self.cursor.row = self.rows - 1;
            }
        }
    }

    /// Scroll terminal up by one line
    fn scroll_up(&mut self) {
        // Save first line to scrollback
        if self.scrollback.len() >= self.max_scrollback {
            self.scrollback.remove(0);
        }
        let first_line: Vec<Cell> = (0..self.cols)
            .map(|col| self.grid[col as usize].clone())
            .collect();
        self.scrollback.push(first_line);

        // Shift grid up
        let row_size = self.cols as usize;
        for row in 0..(self.rows as usize - 1) {
            for col in 0..row_size {
                let src_idx = (row + 1) * row_size + col;
                let dst_idx = row * row_size + col;
                self.grid[dst_idx] = self.grid[src_idx].clone();
            }
        }

        // Clear last line
        let last_row = (self.rows - 1) as usize;
        for col in 0..row_size {
            self.grid[last_row * row_size + col] = Cell::default();
        }
    }

    /// Clear line from cursor to end
    fn clear_to_eol(&mut self) {
        let row = self.cursor.row;
        for col in self.cursor.col..self.cols {
            if let Some(cell) = self.cell_mut(col, row) {
                *cell = Cell::default();
            }
        }
    }

    /// Clear line from start to cursor
    fn clear_to_bol(&mut self) {
        let row = self.cursor.row;
        for col in 0..=self.cursor.col {
            if let Some(cell) = self.cell_mut(col, row) {
                *cell = Cell::default();
            }
        }
    }

    /// Clear entire line
    fn clear_line(&mut self) {
        let row = self.cursor.row;
        for col in 0..self.cols {
            if let Some(cell) = self.cell_mut(col, row) {
                *cell = Cell::default();
            }
        }
    }

    /// Clear screen from cursor to end
    fn clear_to_eos(&mut self) {
        self.clear_to_eol();
        for row in (self.cursor.row + 1)..self.rows {
            for col in 0..self.cols {
                if let Some(cell) = self.cell_mut(col, row) {
                    *cell = Cell::default();
                }
            }
        }
    }

    /// Clear screen from start to cursor
    fn clear_to_bos(&mut self) {
        self.clear_to_bol();
        for row in 0..self.cursor.row {
            for col in 0..self.cols {
                if let Some(cell) = self.cell_mut(col, row) {
                    *cell = Cell::default();
                }
            }
        }
    }

    /// Clear entire screen
    fn clear_screen(&mut self) {
        for cell in &mut self.grid {
            *cell = Cell::default();
        }
    }

    /// Reset text attributes
    fn reset_attributes(&mut self) {
        self.current_fg = [229, 229, 229];
        self.current_bg = [30, 30, 30];
        self.current_bold = false;
        self.current_italic = false;
        self.current_underline = false;
        self.current_inverse = false;
    }

    /// Write character speculatively for local echo.
    ///
    /// Only predicts printable ASCII characters. Returns true if the character
    /// was written speculatively, false if it should wait for server response.
    ///
    /// The server is authoritative - when the next frame arrives, it will
    /// overwrite any speculative state, correcting any mispredictions.
    pub fn write_char_speculative(&mut self, c: char) -> bool {
        // Only predict printable ASCII (0x20-0x7E)
        if !c.is_ascii_graphic() && c != ' ' {
            return false;
        }

        // Copy attributes before mutable borrow (same pattern as write_char)
        let fg = if self.current_inverse { self.current_bg } else { self.current_fg };
        let bg = if self.current_inverse { self.current_fg } else { self.current_bg };
        let bold = self.current_bold;
        let italic = self.current_italic;
        let underline = self.current_underline;
        let inverse = self.current_inverse;
        let col = self.cursor.col;
        let row = self.cursor.row;

        if let Some(cell) = self.cell_mut(col, row) {
            cell.c = c;
            cell.fg = fg;
            cell.bg = bg;
            cell.bold = bold;
            cell.italic = italic;
            cell.underline = underline;
            cell.inverse = inverse;
        } else {
            return false;
        }

        // Advance cursor
        self.cursor.col += 1;
        if self.cursor.col >= self.cols {
            self.cursor.col = 0;
            self.cursor.row += 1;
            if self.cursor.row >= self.rows {
                // Don't scroll speculatively - let server handle it
                self.cursor.row = self.rows - 1;
            }
        }

        self.dirty = true;
        true
    }
}

/// VTE Perform implementation for Terminal
impl Perform for Terminal {
    fn print(&mut self, c: char) {
        self.write_char(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            // Bell
            0x07 => {
                tracing::debug!("Bell");
            }
            // Backspace
            0x08 => {
                if self.cursor.col > 0 {
                    self.cursor.col -= 1;
                }
            }
            // Tab
            0x09 => {
                let tab_stop = ((self.cursor.col / 8) + 1) * 8;
                self.cursor.col = tab_stop.min(self.cols - 1);
            }
            // Line feed / Vertical tab / Form feed
            0x0A..=0x0C => {
                self.cursor.row += 1;
                if self.cursor.row >= self.rows {
                    self.scroll_up();
                    self.cursor.row = self.rows - 1;
                }
            }
            // Carriage return
            0x0D => {
                self.cursor.col = 0;
            }
            _ => {}
        }
    }

    fn hook(&mut self, _params: &Params, _intermediates: &[u8], _ignore: bool, _action: char) {}

    fn put(&mut self, _byte: u8) {}

    fn unhook(&mut self) {}

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        // Handle OSC sequences (e.g., window title)
        if params.len() >= 2 {
            match params[0] {
                b"0" | b"2" => {
                    // Set window title
                    if let Ok(title) = std::str::from_utf8(params[1]) {
                        tracing::debug!("Title: {}", title);
                    }
                }
                _ => {}
            }
        }
    }

    fn csi_dispatch(&mut self, params: &Params, _intermediates: &[u8], _ignore: bool, action: char) {
        let params: Vec<u16> = params.iter().map(|p| p.first().copied().unwrap_or(0)).collect();

        match action {
            // Cursor Up
            'A' => {
                let n = params.first().copied().unwrap_or(1).max(1);
                self.cursor.row = self.cursor.row.saturating_sub(n);
            }
            // Cursor Down
            'B' => {
                let n = params.first().copied().unwrap_or(1).max(1);
                self.cursor.row = (self.cursor.row + n).min(self.rows - 1);
            }
            // Cursor Forward
            'C' => {
                let n = params.first().copied().unwrap_or(1).max(1);
                self.cursor.col = (self.cursor.col + n).min(self.cols - 1);
            }
            // Cursor Back
            'D' => {
                let n = params.first().copied().unwrap_or(1).max(1);
                self.cursor.col = self.cursor.col.saturating_sub(n);
            }
            // Cursor Position (CUP)
            'H' | 'f' => {
                let row = params.first().copied().unwrap_or(1).max(1) - 1;
                let col = params.get(1).copied().unwrap_or(1).max(1) - 1;
                self.cursor.row = row.min(self.rows - 1);
                self.cursor.col = col.min(self.cols - 1);
            }
            // Erase in Display
            'J' => {
                match params.first().copied().unwrap_or(0) {
                    0 => self.clear_to_eos(),
                    1 => self.clear_to_bos(),
                    2 | 3 => self.clear_screen(),
                    _ => {}
                }
            }
            // Erase in Line
            'K' => {
                match params.first().copied().unwrap_or(0) {
                    0 => self.clear_to_eol(),
                    1 => self.clear_to_bol(),
                    2 => self.clear_line(),
                    _ => {}
                }
            }
            // Select Graphic Rendition (SGR)
            'm' => {
                if params.is_empty() {
                    self.reset_attributes();
                    return;
                }

                let mut i = 0;
                while i < params.len() {
                    match params[i] {
                        0 => self.reset_attributes(),
                        1 => self.current_bold = true,
                        3 => self.current_italic = true,
                        4 => self.current_underline = true,
                        7 => self.current_inverse = true,
                        22 => self.current_bold = false,
                        23 => self.current_italic = false,
                        24 => self.current_underline = false,
                        27 => self.current_inverse = false,
                        // Foreground colors
                        30 => self.current_fg = [0, 0, 0],
                        31 => self.current_fg = [205, 49, 49],
                        32 => self.current_fg = [13, 188, 121],
                        33 => self.current_fg = [229, 229, 16],
                        34 => self.current_fg = [36, 114, 200],
                        35 => self.current_fg = [188, 63, 188],
                        36 => self.current_fg = [17, 168, 205],
                        37 => self.current_fg = [229, 229, 229],
                        39 => self.current_fg = [229, 229, 229], // Default
                        // Bright foreground
                        90 => self.current_fg = [102, 102, 102],
                        91 => self.current_fg = [241, 76, 76],
                        92 => self.current_fg = [35, 209, 139],
                        93 => self.current_fg = [245, 245, 67],
                        94 => self.current_fg = [59, 142, 234],
                        95 => self.current_fg = [214, 112, 214],
                        96 => self.current_fg = [41, 184, 219],
                        97 => self.current_fg = [255, 255, 255],
                        // Background colors
                        40 => self.current_bg = [0, 0, 0],
                        41 => self.current_bg = [205, 49, 49],
                        42 => self.current_bg = [13, 188, 121],
                        43 => self.current_bg = [229, 229, 16],
                        44 => self.current_bg = [36, 114, 200],
                        45 => self.current_bg = [188, 63, 188],
                        46 => self.current_bg = [17, 168, 205],
                        47 => self.current_bg = [229, 229, 229],
                        49 => self.current_bg = [30, 30, 30], // Default
                        // Bright background
                        100 => self.current_bg = [102, 102, 102],
                        101 => self.current_bg = [241, 76, 76],
                        102 => self.current_bg = [35, 209, 139],
                        103 => self.current_bg = [245, 245, 67],
                        104 => self.current_bg = [59, 142, 234],
                        105 => self.current_bg = [214, 112, 214],
                        106 => self.current_bg = [41, 184, 219],
                        107 => self.current_bg = [255, 255, 255],
                        // 256 colors / RGB
                        38 => {
                            if params.len() > i + 2 && params[i + 1] == 5 {
                                // 256 color
                                self.current_fg = color_256(params[i + 2] as u8);
                                i += 2;
                            } else if params.len() > i + 4 && params[i + 1] == 2 {
                                // RGB
                                self.current_fg = [
                                    params[i + 2] as u8,
                                    params[i + 3] as u8,
                                    params[i + 4] as u8,
                                ];
                                i += 4;
                            }
                        }
                        48 => {
                            if params.len() > i + 2 && params[i + 1] == 5 {
                                // 256 color
                                self.current_bg = color_256(params[i + 2] as u8);
                                i += 2;
                            } else if params.len() > i + 4 && params[i + 1] == 2 {
                                // RGB
                                self.current_bg = [
                                    params[i + 2] as u8,
                                    params[i + 3] as u8,
                                    params[i + 4] as u8,
                                ];
                                i += 4;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
            }
            // Save cursor
            's' => {
                self.saved_cursor = self.cursor.clone();
            }
            // Restore cursor
            'u' => {
                self.cursor = self.saved_cursor.clone();
            }
            // Show cursor
            'h' if params.first() == Some(&25) => {
                self.cursor.visible = true;
            }
            // Hide cursor
            'l' if params.first() == Some(&25) => {
                self.cursor.visible = false;
            }
            // Device Status Report (DSR)
            'n' => {
                let code = params.first().copied().unwrap_or(0);
                match code {
                    5 => {
                        // "OK" status
                        self.responses.push_back(b"\x1b[0n".to_vec());
                    }
                    6 => {
                        // Report cursor position (1-based)
                        let row = self.cursor.row + 1;
                        let col = self.cursor.col + 1;
                        let resp = format!("\x1b[{};{}R", row, col);
                        self.responses.push_back(resp.into_bytes());
                    }
                    _ => {}
                }
            }
            _ => {
                tracing::trace!("Unhandled CSI: {} params={:?}", action, params);
            }
        }
    }

    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, byte: u8) {
        match byte {
            // Save cursor (DECSC)
            b'7' => {
                self.saved_cursor = self.cursor.clone();
            }
            // Restore cursor (DECRC)
            b'8' => {
                self.cursor = self.saved_cursor.clone();
            }
            // Reset
            b'c' => {
                self.clear_screen();
                self.cursor = Cursor::default();
                self.reset_attributes();
            }
            _ => {}
        }
    }
}

/// Convert 256-color index to RGB
fn color_256(idx: u8) -> [u8; 3] {
    match idx {
        0 => [0, 0, 0],
        1 => [205, 49, 49],
        2 => [13, 188, 121],
        3 => [229, 229, 16],
        4 => [36, 114, 200],
        5 => [188, 63, 188],
        6 => [17, 168, 205],
        7 => [229, 229, 229],
        8 => [102, 102, 102],
        9 => [241, 76, 76],
        10 => [35, 209, 139],
        11 => [245, 245, 67],
        12 => [59, 142, 234],
        13 => [214, 112, 214],
        14 => [41, 184, 219],
        15 => [255, 255, 255],
        16..=231 => {
            // 6x6x6 color cube
            let idx = idx - 16;
            let r = (idx / 36) * 51;
            let g = ((idx / 6) % 6) * 51;
            let b = (idx % 6) * 51;
            [r, g, b]
        }
        232..=255 => {
            // Grayscale
            let gray = (idx - 232) * 10 + 8;
            [gray, gray, gray]
        }
    }
}
