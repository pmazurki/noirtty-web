# Plan Migracji: WebSocket → WebTransport

## Uzasadnienie decyzji (Zalety WebTransport)

### Dlaczego WebTransport?

| Aspekt | WebSocket (obecny) | WebTransport (cel) |
|--------|-------------------|-------------------|
| **Protokół** | TCP | QUIC (UDP) |
| **Opóźnienie** | Wyższe (TCP handshake, head-of-line blocking) | **Niższe** (0-RTT, brak HOL blocking) |
| **Multipleksowanie** | Brak (1 połączenie = 1 strumień) | **Wiele strumieni** na jednym połączeniu |
| **Odporność na straty** | Blokuje przy utracie pakietu | **Izolowane strumienie** (utrata nie blokuje innych) |
| **Mobilność** | Zrywa przy zmianie sieci | **Connection migration** (QUIC) |

### Kluczowe korzyści dla terminala webowego:

1. **Niższe opóźnienie klawiatury** - krytyczne dla interaktywnej pracy w terminalu
2. **Lepsze działanie na słabych sieciach** (WiFi, mobile) - brak blokowania przy utracie pakietów
3. **Szybsze reconnect** po zmianie sieci (przełączenie WiFi ↔ LTE)
4. **Możliwość osobnych strumieni** dla input/output (lepsza kontrola QoS)

### Ryzyka i wady:

1. **Wymagany HTTPS** - TLS 1.3 obowiązkowy (utrudnia dev na localhost)
2. **Brak Firefox** - na dziś (2025) tylko Chrome 118+, Safari 17.4+, Edge 118+
3. **Nowsza technologia** - mniej przykładów i dokumentacji
4. **Złożoność** - stream-based I/O wymaga framing layer

---

## Obecny stan projektu

### Gotowe:
- Zależności w `Cargo.toml`: `web-transport-quinn`, `quinn`, `rustls`
- Bindingi WebTransport w `client/Cargo.toml` (web_sys features)
- Obsługa HTTPS z self-signed cert (`--https` flag)
- Czysta abstrakcja transportu w `client/src/transport.rs`

### Do zmiany:
- `server/src/main.rs` - obecnie tylko WebSocket
- `client/src/transport.rs` - przepisać na WebTransport API
- `static/index.html` - zmienić logikę połączenia (linie ~1093-1135)

---

## Plan implementacji

### Faza 1: Serwer WebTransport (2 dni)

**Plik:** `server/src/main.rs`

1. **Dodać endpoint WebTransport** na porcie 4433 (QUIC)
   - Użyć `web-transport-quinn` do obsługi połączeń
   - Uruchomić równolegle z HTTP server (port 3000)

2. **Obsługa strumieni bidirectional**
   ```rust
   // Nowy handler dla WebTransport
   async fn handle_webtransport_session(session: WebTransportSession) {
       let stream = session.accept_bi().await?;
       // ... obsługa input/output
   }
   ```

3. **Framing protocol** - dodać length-prefix dla wiadomości:
   ```
   [4 bytes: length][N bytes: JSON message]
   ```

4. **Sesje** - reużyć istniejący `DashMap<String, Session>`

### Faza 2: Protokół wiadomości (0.5 dnia)

**Pliki:** `server/src/main.rs`, `client/src/transport.rs`

1. Dodać moduł `framing.rs` z funkcjami:
   - `encode_message(msg: &ServerMessage) -> Vec<u8>`
   - `decode_message(bytes: &[u8]) -> ClientMessage`

2. Zachować JSON jako format (prostota) lub opcjonalnie przejść na `rkyv` (szybkość)

### Faza 3: Client WebTransport (1 dzień)

**Plik:** `client/src/transport.rs`

1. **Nowa struktura Transport:**
   ```rust
   pub struct Transport {
       transport: web_sys::WebTransport,
       reader: ReadableStreamDefaultReader,
       writer: WritableStreamDefaultWriter,
       recv_buffer: Rc<RefCell<VecDeque<TerminalFrame>>>,
   }
   ```

2. **Metoda connect:**
   ```rust
   pub async fn connect(url: &str) -> Result<Self, JsValue> {
       let transport = web_sys::WebTransport::new(url)?;
       transport.ready().await?;
       let stream = transport.create_bidirectional_stream().await?;
       // setup reader/writer...
   }
   ```

3. **Zachować API:** `send()`, `send_resize()`, `try_recv()`, `ready_state()`

### Faza 4: Integracja HTML/JS (0.5 dnia)

**Plik:** `static/index.html`

1. Zmienić URL połączenia:
   ```javascript
   // Przed (WebSocket):
   const wsUrl = `wss://${window.location.host}/ws?session=${sessionId}`;

   // Po (WebTransport):
   const wtUrl = `https://${window.location.hostname}:4433/wt?session=${sessionId}`;
   ```

2. Feature detection + fallback:
   ```javascript
   if (typeof WebTransport !== 'undefined') {
       // użyj WebTransport
   } else {
       // fallback do WebSocket
   }
   ```

### Faza 5: Testy i polish (1 dzień)

1. **Testy manualne:**
   - Połączenie, rozłączenie, reconnect
   - Typing latency (subiektywnie)
   - Resize terminala
   - Scroll

2. **Testy na różnych przeglądarkach:**
   - Safari 17.4+ (macOS/iPadOS)
   - Chrome 118+
   - Edge 118+

3. **Network conditions:**
   - Chrome DevTools → Network throttling
   - Packet loss simulation

---

## Pliki do modyfikacji

| Plik | Zmiany |
|------|--------|
| `server/src/main.rs` | Dodać WebTransport server na porcie 4433 |
| `server/src/framing.rs` | **Nowy plik** - length-prefix encoding |
| `client/src/transport.rs` | Przepisać na WebTransport API |
| `client/src/framing.rs` | **Nowy plik** - length-prefix decoding |
| `static/index.html` | Zmienić connect(), dodać feature detection |
| `Makefile` | Dodać target `run-webtransport` |

---

## Weryfikacja (Definition of Done)

1. `make run --https` uruchamia serwer z WebTransport na :4433
2. Terminal działa w Safari 17.4+ i Chrome 118+
3. Latency keyboard input < 50ms (subiektywnie responsywny)
4. Reconnect działa po rozłączeniu sieci
5. Fallback do WebSocket działa w Firefox

---

## Alternatywa: Zachować WebSocket

Jeśli zdecydujesz, że WebTransport nie jest teraz potrzebny:
- Obecna implementacja WebSocket działa
- Mniejsza złożoność
- Pełne wsparcie przeglądarek
- Można wrócić do WebTransport później

**Rekomendacja:** Wdrożyć WebTransport z fallback do WebSocket - najlepsze z obu światów.
