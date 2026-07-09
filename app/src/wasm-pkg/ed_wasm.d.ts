/* tslint:disable */
/* eslint-disable */

export class EditorSession {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Dispatch a JSON command; returns `{"ok":true,...}` or `{"error":..}`.
     */
    command(json: string): string;
    /**
     * PNG flavor for system-clipboard copy (spec §10.7).
     */
    copy_as_png(): Uint8Array;
    /**
     * Export an artboard; empty vec on failure.
     */
    export_artboard(artboard: number, scale: number, format: string, background: boolean, quality: number): Uint8Array;
    frame_len(): number;
    frame_ptr(): number;
    /**
     * `scale` < 0 fits the artboard; `new_doc` opens as its own document.
     */
    import_image(bytes: Uint8Array, name: string, scale: number, new_doc: boolean): string;
    /**
     * True when the document/view changed since the last rendered frame.
     */
    needs_frame(): boolean;
    constructor();
    open_myed(bytes: Uint8Array, name: string): string;
    /**
     * Render the viewport; frame bytes stay in wasm memory.
     */
    render(width: number, height: number, ants_phase: number): void;
    save_myed(): Uint8Array;
    /**
     * Full UI state mirror (spec §12.1 read models).
     */
    state(): string;
}

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_editorsession_free: (a: number, b: number) => void;
    readonly editorsession_command: (a: number, b: number, c: number) => [number, number];
    readonly editorsession_copy_as_png: (a: number) => [number, number];
    readonly editorsession_export_artboard: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => [number, number];
    readonly editorsession_frame_len: (a: number) => number;
    readonly editorsession_frame_ptr: (a: number) => number;
    readonly editorsession_import_image: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => [number, number];
    readonly editorsession_needs_frame: (a: number) => number;
    readonly editorsession_new: () => number;
    readonly editorsession_open_myed: (a: number, b: number, c: number, d: number, e: number) => [number, number];
    readonly editorsession_render: (a: number, b: number, c: number, d: number) => void;
    readonly editorsession_save_myed: (a: number) => [number, number];
    readonly editorsession_state: (a: number) => [number, number];
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
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
