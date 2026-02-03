/* tslint:disable */
/* eslint-disable */

/**
 * Main terminal application
 */
export class NoirTTYWeb {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Get terminal cols
     */
    cols(): number;
    /**
     * Connect to WebTransport server
     */
    connect(url: string): Promise<void>;
    /**
     * WebSocket connection state (0=connecting,1=open,2=closing,3=closed)
     */
    connection_state(): number;
    /**
     * Copy selection to clipboard
     */
    copy_selection(): string | undefined;
    /**
     * Debug: get a single cell with fg/bg info.
     */
    debug_cell(col: number, row: number): string;
    /**
     * Debug: get text of a row for quick inspection.
     */
    debug_row(row: number): string;
    /**
     * Debug: number of text layout runs in the renderer.
     */
    debug_text_runs(): number;
    /**
     * Number of frames received from the server.
     */
    frame_count(): bigint;
    /**
     * Initialize the WebGPU renderer
     */
    init_renderer(canvas_id: string): Promise<void>;
    /**
     * Maximum surface dimension supported by the active renderer.
     */
    max_surface_dim(): number;
    /**
     * Create a new NoirTTY terminal instance
     */
    constructor(_canvas_id: string);
    /**
     * Handle keyboard event
     */
    on_key(code: string, key: string, ctrl: boolean, alt: boolean, meta: boolean, shift: boolean): void;
    /**
     * Handle mouse down
     */
    on_mouse_down(x: number, y: number): void;
    /**
     * Handle mouse move
     */
    on_mouse_move(x: number, y: number): void;
    /**
     * Handle mouse up
     */
    on_mouse_up(): void;
    /**
     * Paste from clipboard
     */
    paste(text: string): void;
    /**
     * Render frame - call from requestAnimationFrame
     */
    render(): void;
    /**
     * Get current renderer type ("webgpu", "canvas2d", "uninitialized")
     */
    renderer_type(): string;
    /**
     * Resize terminal
     */
    resize(cols: number, rows: number): void;
    /**
     * Get terminal rows
     */
    rows(): number;
    /**
     * Scroll terminal viewport (positive = scroll up)
     */
    scroll(delta: number): void;
    /**
     * Send input to terminal
     */
    send_input(data: string): void;
    /**
     * Debug: render a fixed test string instead of terminal content.
     */
    set_debug_text(enabled: boolean): void;
    /**
     * Throttle server frame rate (0 = no throttle).
     */
    set_frame_throttle_ms(min_interval_ms: number): void;
    /**
     * Limit the number of frames kept in the client queue (0 = unlimited).
     */
    set_max_frames_in_queue(max_frames: number): void;
    /**
     * Configure renderer (font + colors)
     */
    set_render_config(font_size: number, font_stack: string, background: string, selection: string, cursor: string, cursor_text: string): void;
    /**
     * Total bytes received by transport.
     */
    transport_bytes_received(): bigint;
    /**
     * Total messages received by transport.
     */
    transport_messages_received(): bigint;
    /**
     * Number of frames queued in the client transport.
     */
    transport_queue_len(): number;
    /**
     * Reset transport counters.
     */
    transport_reset_counters(): void;
    /**
     * Update size based on available dimensions (pixels)
     */
    update_size(width: number, height: number): void;
}

/**
 * Initialize panic hook for better WASM debugging
 */
export function init(): void;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_noirttyweb_free: (a: number, b: number) => void;
    readonly noirttyweb_cols: (a: number) => number;
    readonly noirttyweb_connect: (a: number, b: number, c: number) => any;
    readonly noirttyweb_connection_state: (a: number) => number;
    readonly noirttyweb_copy_selection: (a: number) => [number, number];
    readonly noirttyweb_debug_cell: (a: number, b: number, c: number) => [number, number];
    readonly noirttyweb_debug_row: (a: number, b: number) => [number, number];
    readonly noirttyweb_debug_text_runs: (a: number) => number;
    readonly noirttyweb_frame_count: (a: number) => bigint;
    readonly noirttyweb_init_renderer: (a: number, b: number, c: number) => any;
    readonly noirttyweb_max_surface_dim: (a: number) => number;
    readonly noirttyweb_new: (a: number, b: number) => [number, number, number];
    readonly noirttyweb_on_key: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number) => [number, number];
    readonly noirttyweb_on_mouse_down: (a: number, b: number, c: number) => void;
    readonly noirttyweb_on_mouse_move: (a: number, b: number, c: number) => void;
    readonly noirttyweb_on_mouse_up: (a: number) => void;
    readonly noirttyweb_paste: (a: number, b: number, c: number) => [number, number];
    readonly noirttyweb_render: (a: number) => [number, number];
    readonly noirttyweb_renderer_type: (a: number) => [number, number];
    readonly noirttyweb_resize: (a: number, b: number, c: number) => [number, number];
    readonly noirttyweb_rows: (a: number) => number;
    readonly noirttyweb_scroll: (a: number, b: number) => [number, number];
    readonly noirttyweb_send_input: (a: number, b: number, c: number) => [number, number];
    readonly noirttyweb_set_debug_text: (a: number, b: number) => void;
    readonly noirttyweb_set_frame_throttle_ms: (a: number, b: number) => [number, number];
    readonly noirttyweb_set_max_frames_in_queue: (a: number, b: number) => void;
    readonly noirttyweb_set_render_config: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number) => [number, number];
    readonly noirttyweb_transport_bytes_received: (a: number) => bigint;
    readonly noirttyweb_transport_messages_received: (a: number) => bigint;
    readonly noirttyweb_transport_queue_len: (a: number) => number;
    readonly noirttyweb_transport_reset_counters: (a: number) => void;
    readonly noirttyweb_update_size: (a: number, b: number, c: number) => [number, number];
    readonly init: () => void;
    readonly wasm_bindgen__closure__destroy__h2abc3dcfced14ffd: (a: number, b: number) => void;
    readonly wasm_bindgen__closure__destroy__heb3e6ab2f89f4149: (a: number, b: number) => void;
    readonly wasm_bindgen__convert__closures_____invoke__hddb92fdd4f3e2dde: (a: number, b: number, c: any, d: any) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h8c965f4e6ad2e967: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen__convert__closures_____invoke__he98c29023c3f020e: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h28794dc364ac4eea: (a: number, b: number) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
