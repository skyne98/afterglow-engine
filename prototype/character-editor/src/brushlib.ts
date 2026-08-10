/* Brushlib NG wasm — typing shim for the emscripten-generated module. */
export interface BrushModule {
  ccall: (...args: unknown[]) => number;
  HEAPF32: Float32Array;
  HEAPU8: Uint8Array;
  HEAPU16: Uint16Array;
  HEAP32: Int32Array;
  _init: (width: number, height: number) => number;
  _paint_destroy: () => number;
  _new_brush: () => number;
  _load_brush: (json: number) => number;
  _reset_brush: () => number;
  _stroke_to: (...args: number[]) => number;
  _set_brush_base_value: (...args: number[]) => number;
  _get_brush_base_value: (...args: number[]) => number;
  _set_brush_mapping_n: (...args: number[]) => number;
  _set_brush_mapping_point: (...args: number[]) => number;
  _begin_stroke: (...args: number[]) => number;
  _paint_begin_atomic: () => number;
  _paint_end_atomic: () => number;
  _paint_begin_batch: () => number;
  _paint_end_batch: () => number;
  _paint_get_width: () => number;
  _paint_get_height: () => number;
  _paint_get_error_code: () => number;
  _paint_clear_error: () => number;
  _paint_get_tiles_width: () => number;
  _paint_get_tiles_height: () => number;
  _paint_get_used_tile_count: () => number;
  _paint_get_tile_ptr: (tx: number, ty: number) => number;
  _paint_render_tile_ptr: (tx: number, ty: number) => number;
  _paint_set_eotf: (eotf: number) => number;
  _paint_render_rgba8_tile_ptr: (tx: number, ty: number) => number;
  _paint_render_layer_rgba8_tile_ptr: (layerId: number, tx: number, ty: number) => number;
  _paint_write_rgba8_tile: (tx: number, ty: number, sourcePtr: number) => number;
  _paint_render_rgba8_mip_tile_ptr: (tx: number, ty: number, level: number) => number;
  _paint_region_has_paint: (tx: number, ty: number, level: number) => number;
  _paint_get_dirty_count: () => number;
  _paint_get_dirty_rect: (index: number, outPtr: number) => number;
  _paint_clear_dirty: () => number;
  _paint_set_background_color: (r: number, g: number, b: number) => number;
  _paint_clear_background: () => number;
  _paint_history_begin: () => number;
  _paint_history_commit: () => number;
  _paint_history_undo: () => number;
  _paint_history_redo: () => number;
  _paint_history_can_undo: () => number;
  _paint_history_can_redo: () => number;
  _paint_clear: () => number;
  _paint_pick_color: (...args: number[]) => number;
  _paint_set_symmetry: (...args: number[]) => number;
  _paint_get_layer_count: () => number;
  _paint_get_active_layer: () => number;
  _paint_set_active_layer: (layerId: number) => number;
  _paint_create_layer: () => number;
  _paint_delete_layer: (layerId: number) => number;
  _paint_set_layer_visible: (layerId: number, visible: number) => number;
  _paint_set_layer_opacity: (layerId: number, opacity: number) => number;
  _paint_get_layer_opacity?: (layerId: number) => number;
  _paint_get_layer_mode: (layerId: number) => number;
  _paint_set_layer_mode: (layerId: number, mode: number) => number;
  _paint_get_layer_visible: (layerId: number) => number;
  _paint_get_layer_group: (layerId: number) => number;
  _paint_set_layer_group: (layerId: number, groupId: number) => number;
  _paint_move_layer: (layerId: number, direction: number) => number;
  _paint_get_group_count: () => number;
  _paint_get_group_alive: (groupId: number) => number;
  _paint_get_group_parent: (groupId: number) => number;
  _paint_create_group: () => number;
  _paint_delete_group: (groupId: number) => number;
  _paint_set_group_parent: (groupId: number, parentId: number) => number;
  _paint_get_group_visible: (groupId: number) => number;
  _paint_set_group_visible: (groupId: number, visible: number) => number;
  _paint_get_group_opacity: (groupId: number) => number;
  _paint_set_group_opacity: (groupId: number, opacity: number) => number;
  _paint_get_group_mode: (groupId: number) => number;
  _paint_set_group_mode: (groupId: number, mode: number) => number;
  _paint_get_group_pass_through: (groupId: number) => number;
  _paint_set_group_pass_through: (groupId: number, value: number) => number;
  _paint_get_group_isolated: (groupId: number) => number;
  _paint_set_group_isolated: (groupId: number, value: number) => number;
  _paint_move_group: (groupId: number, direction: number) => number;
  _malloc: (size: number) => number;
  _free: (ptr: number) => void;
  stringToUTF8: (value: string, ptr: number, maxBytes: number) => void;
  lengthBytesUTF8: (value: string) => number;
  getValue: (ptr: number, type: string) => number;
  setValue: (ptr: number, value: number, type: string) => void;
  cwrap: (...args: unknown[]) => unknown;
}

let loadPromise: Promise<BrushModule> | null = null;

/** Load the brushlib wasm module (created with MODULARIZE + UMD). */
export function loadBrushModule(): Promise<BrushModule> {
  if (loadPromise) return loadPromise;
  loadPromise = (async () => {
    // The emscripten UMD module exports the Module factory as module.exports
    // (a function). Vite's CJS interop may wrap it.
    // Hide the dynamic import from Vite's static analysis (avoids the
    // worker-import-meta-url plugin error on Emscripten's generated JS).
    // The module is served from public/wasm/ at runtime.
    const dynamicImport = new Function('url', 'return import(url)') as (url: string) => Promise<any>;
    const mod: any = await dynamicImport('/wasm/brushlib.js');
    const factory =
      typeof mod === 'function' ? mod :
      typeof mod.default === 'function' ? mod.default :
      mod.default ?? mod;
    const module = await factory({
      locateFile: (_path: string) => `/wasm/${_path}`,
    });
    return module as BrushModule;
  })();
  return loadPromise;
}
