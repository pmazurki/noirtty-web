# Plan: WebSocket + Local Echo

## Czym jest Local Echo?

Local Echo = natychmiastowe wyświetlanie wpisywanych znaków **przed** odpowiedzią serwera.

```
BEZ Local Echo (obecny stan):
  key press → WebSocket → Server → PTY → Server → WebSocket → display
  [=========== 20-150ms latency ===========]

Z Local Echo:
  key press → [instant local display] → WebSocket → Server → validate
  [0ms]      [=========== background ===========]
```

---

## Obecny stan projektu (DOBRA WIADOMOŚĆ)

### Klient JUŻ MA potrzebną infrastrukturę:

1. **Pełny stan terminala** (`client/src/terminal.rs`):
   - `grid: Vec<Cell>` - pełna siatka znaków
   - `cursor: Cursor` - pozycja kursora
   - `current_fg/bg` - atrybuty tekstu

2. **Parser VTE** (`vte` crate):
   - Zaimplementowany trait `Perform` (linie 480-740)
   - Obsługa CSI sequences, kolorów, atrybutów
   - **ALE: obecnie NIEUŻYWANY!**

3. **Problem**: Serwer nadpisuje całą siatkę:
   ```rust
   // terminal.rs:164
   self.grid = frame.cells;  // Całkowite nadpisanie!
   ```

---

## Strategia implementacji

### Opcja A: Conservative (REKOMENDOWANA)

**Zakres:**
- Tylko znaki drukowalne (a-z, A-Z, 0-9, spacja, interpunkcja)
- Pomiń escape sequences, control chars
- ~500 LOC

**Zachowanie:**
1. User wpisuje "a"
2. Natychmiast: wyświetl "a" na pozycji kursora, przesuń kursor
3. W tle: wyślij do serwera
4. Gdy przyjdzie ramka serwera: zwaliduj (powinno się zgadzać)

### Opcja B: Moderate

**Zakres dodatkowo:**
- Backspace
- Enter
- Strzałki (ruch kursora)
- Ctrl+C, Ctrl+D
- ~1500 LOC

### Opcja C: Aggressive (NIE REKOMENDOWANA)

- Pełna predykcja VTE
- Bardzo złożone, ryzyko "ghost characters"

---

## Plan implementacji (Opcja A)

### Faza 1: Speculative write (0.5 dnia)

**Plik:** `client/src/terminal.rs`

Dodać metodę do spekulatywnego zapisu:

```rust
impl Terminal {
    /// Write character speculatively (local echo)
    pub fn write_char_speculative(&mut self, c: char) -> bool {
        // Only predict printable ASCII
        if !c.is_ascii_graphic() && c != ' ' {
            return false;  // Don't predict
        }

        // Write char at cursor
        let idx = self.cursor.row as usize * self.cols as usize
                + self.cursor.col as usize;
        if idx < self.grid.len() {
            self.grid[idx] = Cell {
                c,
                fg: self.current_fg,
                bg: self.current_bg,
                bold: self.bold,
                // ... other attributes
            };
        }

        // Advance cursor
        self.cursor.col += 1;
        if self.cursor.col >= self.cols {
            self.cursor.col = 0;
            self.cursor.row += 1;
            // Handle scroll if needed
        }

        true  // Predicted successfully
    }
}
```

### Faza 2: Input handler integration (0.5 dnia)

**Plik:** `client/src/lib.rs`

Zmodyfikować `on_key()`:

```rust
pub fn on_key(&mut self, code: &str, key: &str, ...) -> Result<(), JsValue> {
    let data = self.input.handle_key(code, key, ...);

    if let Some(ref data) = data {
        // LOCAL ECHO: Try to predict simple characters
        if data.len() == 1 {
            let c = data.chars().next().unwrap();
            if self.terminal.write_char_speculative(c) {
                // Mark as dirty for immediate render
                self.needs_render = true;
            }
        }

        // Send to server (always)
        self.transport.send(data.as_bytes())?;
    }

    Ok(())
}
```

### Faza 3: Frame reconciliation (0.5 dnia)

**Plik:** `client/src/terminal.rs`

Zmodyfikować `apply_frame()` dla walidacji:

```rust
pub fn apply_frame(&mut self, frame: TerminalFrame) {
    // Server is authoritative - apply frame
    // Any speculative writes are automatically corrected

    if frame.cells.len() == expected {
        self.grid = frame.cells;
    }

    self.cursor.col = frame.cursor_col;
    self.cursor.row = frame.cursor_row;
    self.cursor.visible = frame.cursor_visible;

    // Note: If our prediction was wrong, this corrects it
    // Visual glitch is minimal (one frame)
}
```

### Faza 4: Render optimization (0.5 dnia)

**Plik:** `client/src/lib.rs`

Zapewnić natychmiastowy render po local echo:

```rust
pub fn tick(&mut self) -> Result<(), JsValue> {
    // Process server frames
    while let Some(frame) = self.transport.try_recv() {
        self.terminal.apply_frame(frame);
        self.needs_render = true;
    }

    // Render if needed (includes speculative writes)
    if self.needs_render {
        self.renderer.render(&self.terminal)?;
        self.needs_render = false;
    }

    Ok(())
}
```

---

## Pliki do modyfikacji

| Plik | Zmiany |
|------|--------|
| `client/src/terminal.rs` | Dodać `write_char_speculative()` |
| `client/src/lib.rs` | Zmodyfikować `on_key()` dla local echo |
| `client/src/lib.rs` | Upewnić się o natychmiastowym renderze |

---

## Ryzyka i mitigacje

| Ryzyko | Mitigacja |
|--------|-----------|
| **Ghost characters** (złe przewidywanie) | Serwer koryguje w następnej ramce (<16ms) |
| **Programy TUI** (vim, less) | Zazwyczaj wyłączają echo - nasze przewidywanie nie zaszkodzi |
| **Escape sequences** | Nie przewidujemy - czekamy na serwer |
| **Podwójne echo** | Niemożliwe - serwer nadpisuje stan |

---

## Weryfikacja

1. Wpisz "hello world" - znaki pojawiają się natychmiast
2. Backspace - czeka na serwer (nie przewidujemy)
3. Uruchom `vim` - działa normalnie (vim wyłącza echo)
4. Test na wolnym połączeniu (DevTools throttling) - widoczna różnica

---

## Porównanie opcji transportu

| Rozwiązanie | Latency | Złożoność | Browser support |
|-------------|---------|-----------|-----------------|
| **WebSocket (obecny)** | 20-150ms | Niska | Pełne |
| **WebSocket + Local Echo** | **~0ms perceived** | Niska | Pełne |
| **WebTransport** | 10-50ms | Średnia | Brak Firefox |
| **WebRTC** | 10-50ms | Wysoka | Pełne |

**Rekomendacja:** WebSocket + Local Echo daje najlepszy stosunek korzyści do złożoności.

---

## Podsumowanie

Local Echo to **najłatwiejsze i najskuteczniejsze** rozwiązanie problemu latency:
- Nie wymaga zmiany protokołu (WebSocket zostaje)
- Wykorzystuje istniejącą infrastrukturę klienta
- ~500 LOC zmian
- Natychmiastowy feedback dla użytkownika
- Pełna kompatybilność przeglądarek
