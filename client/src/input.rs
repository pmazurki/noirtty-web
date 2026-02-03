//! Keyboard input handling
//!
//! Converts JavaScript keyboard events to terminal escape sequences.

/// Input handler for keyboard events
pub struct InputHandler {
    /// Application cursor keys mode (DECCKM)
    application_cursor_keys: bool,
}

impl InputHandler {
    /// Create a new input handler
    pub fn new() -> Self {
        InputHandler {
            application_cursor_keys: false,
        }
    }

    /// Set application cursor keys mode
    pub fn set_application_cursor_keys(&mut self, enabled: bool) {
        self.application_cursor_keys = enabled;
    }

    /// Process a key event and return the bytes to send to the terminal
    pub fn process_key(
        &self,
        code: &str,
        key: &str,
        ctrl: bool,
        alt: bool,
        meta: bool,
        shift: bool,
    ) -> Option<String> {
        // Handle Ctrl+key combinations
        if ctrl && !alt && !meta {
            if let Some(c) = self.ctrl_key(key) {
                return Some(c.to_string());
            }
        }

        // Handle special keys
        match code {
            // Arrow keys
            "ArrowUp" => return Some(self.arrow_key('A', ctrl, alt, shift)),
            "ArrowDown" => return Some(self.arrow_key('B', ctrl, alt, shift)),
            "ArrowRight" => return Some(self.arrow_key('C', ctrl, alt, shift)),
            "ArrowLeft" => return Some(self.arrow_key('D', ctrl, alt, shift)),

            // Navigation keys
            "Home" => return Some(self.special_key('H', ctrl, alt, shift)),
            "End" => return Some(self.special_key('F', ctrl, alt, shift)),
            "PageUp" => return Some("\x1b[5~".to_string()),
            "PageDown" => return Some("\x1b[6~".to_string()),
            "Insert" => return Some("\x1b[2~".to_string()),
            "Delete" => return Some("\x1b[3~".to_string()),

            // Function keys
            "F1" => return Some("\x1bOP".to_string()),
            "F2" => return Some("\x1bOQ".to_string()),
            "F3" => return Some("\x1bOR".to_string()),
            "F4" => return Some("\x1bOS".to_string()),
            "F5" => return Some("\x1b[15~".to_string()),
            "F6" => return Some("\x1b[17~".to_string()),
            "F7" => return Some("\x1b[18~".to_string()),
            "F8" => return Some("\x1b[19~".to_string()),
            "F9" => return Some("\x1b[20~".to_string()),
            "F10" => return Some("\x1b[21~".to_string()),
            "F11" => return Some("\x1b[23~".to_string()),
            "F12" => return Some("\x1b[24~".to_string()),

            // Control keys
            "Enter" | "NumpadEnter" => return Some("\r".to_string()),
            "Backspace" => {
                return Some(if ctrl { "\x08" } else { "\x7f" }.to_string());
            }
            "Tab" => {
                return Some(if shift { "\x1b[Z" } else { "\t" }.to_string());
            }
            "Escape" => return Some("\x1b".to_string()),

            _ => {}
        }

        // Regular printable character
        if key.len() == 1 && !ctrl && !meta {
            let c = key.chars().next().unwrap();

            // Alt+key sends ESC + key
            if alt {
                return Some(format!("\x1b{}", c));
            }

            return Some(c.to_string());
        }

        None
    }

    /// Convert Ctrl+key to control character
    fn ctrl_key(&self, key: &str) -> Option<char> {
        if key.len() != 1 {
            return None;
        }

        let c = key.chars().next().unwrap().to_ascii_uppercase();
        match c {
            'A'..='Z' => Some((c as u8 - b'A' + 1) as char),
            '@' => Some('\x00'),
            '[' => Some('\x1b'),
            '\\' => Some('\x1c'),
            ']' => Some('\x1d'),
            '^' => Some('\x1e'),
            '_' => Some('\x1f'),
            '?' => Some('\x7f'),
            _ => None,
        }
    }

    /// Generate arrow key sequence
    fn arrow_key(&self, direction: char, ctrl: bool, alt: bool, shift: bool) -> String {
        let modifier = self.modifier_code(ctrl, alt, shift);

        if self.application_cursor_keys && modifier == 0 {
            // Application mode: ESC O <dir>
            format!("\x1bO{}", direction)
        } else if modifier > 0 {
            // With modifiers: ESC [ 1 ; <mod> <dir>
            format!("\x1b[1;{}{}", modifier, direction)
        } else {
            // Normal mode: ESC [ <dir>
            format!("\x1b[{}", direction)
        }
    }

    /// Generate special key sequence (Home, End, etc.)
    fn special_key(&self, key: char, ctrl: bool, alt: bool, shift: bool) -> String {
        let modifier = self.modifier_code(ctrl, alt, shift);

        if modifier > 0 {
            format!("\x1b[1;{}{}", modifier, key)
        } else {
            format!("\x1b[{}", key)
        }
    }

    /// Calculate modifier code for CSI sequences
    fn modifier_code(&self, ctrl: bool, alt: bool, shift: bool) -> u8 {
        let mut modifier = 1u8;
        if shift {
            modifier += 1;
        }
        if alt {
            modifier += 2;
        }
        if ctrl {
            modifier += 4;
        }
        if modifier == 1 {
            0
        } else {
            modifier
        }
    }
}

impl Default for InputHandler {
    fn default() -> Self {
        Self::new()
    }
}
