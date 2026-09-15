export interface PaintLayerInfo {
  id: number;
  active: boolean;
  visible: number;
  opacity: number;
  mode: number;
  group: number;
}

export interface PaintGroupInfo {
  id: number;
  alive: boolean;
  parent: number;
  visible: number;
  opacity: number;
  mode: number;
  passThrough: number;
  isolated: number;
}

export interface PaintLayerState {
  activeLayer: number;
  layers: PaintLayerInfo[];
  groups: PaintGroupInfo[];
}

export function samePaintLayerState(a: PaintLayerState | null, b: PaintLayerState): boolean {
  if (!a || a.activeLayer !== b.activeLayer || a.layers.length !== b.layers.length || a.groups.length !== b.groups.length) return false;
  for (let i = 0; i < a.layers.length; i++) {
    const x = a.layers[i], y = b.layers[i];
    if (x.id !== y.id || x.active !== y.active || x.visible !== y.visible ||
        x.opacity !== y.opacity || x.mode !== y.mode || x.group !== y.group) return false;
  }
  for (let i = 0; i < a.groups.length; i++) {
    const x = a.groups[i], y = b.groups[i];
    if (x.id !== y.id || x.alive !== y.alive || x.parent !== y.parent ||
        x.visible !== y.visible || x.opacity !== y.opacity || x.mode !== y.mode ||
        x.passThrough !== y.passThrough || x.isolated !== y.isolated) return false;
  }
  return true;
}

export type PaintLayerRow =
  | { kind: 'layer'; id: number; depth: number; info: PaintLayerInfo }
  | { kind: 'group'; id: number; depth: number; info: PaintGroupInfo };

export function buildPaintLayerRows(layers: PaintLayerInfo[], groups: PaintGroupInfo[]): PaintLayerRow[] {
  const rows: PaintLayerRow[] = [];
  const alive = new Set(groups.filter((group) => group.alive).map((group) => group.id));
  const visited = new Set<number>();

  const addLayers = (group: number, depth: number) => {
    for (let index = layers.length - 1; index >= 0; index--) {
      const layer = layers[index];
      if (layer.group === group || (group === -1 && !alive.has(layer.group))) {
        rows.push({ kind: 'layer', id: layer.id, depth, info: layer });
      }
    }
  };

  const addGroup = (group: PaintGroupInfo, depth: number) => {
    if (visited.has(group.id)) return;
    visited.add(group.id);
    rows.push({ kind: 'group', id: group.id, depth, info: group });
    for (let index = groups.length - 1; index >= 0; index--) {
      const child = groups[index];
      if (child.alive && child.parent === group.id) addGroup(child, depth + 1);
    }
    addLayers(group.id, depth + 1);
  };

  for (let index = groups.length - 1; index >= 0; index--) {
    const group = groups[index];
    if (group.alive && !alive.has(group.parent)) addGroup(group, 0);
  }
  for (let index = groups.length - 1; index >= 0; index--) {
    const group = groups[index];
    if (group.alive && !visited.has(group.id)) addGroup(group, 0);
  }
  addLayers(-1, 0);
  return rows;
}
