// TS mirrors of the core's read models (spec §12.1). These are plain
// projections — the document itself never crosses into JS.

export type ToolKind =
  | 'select'
  | 'rect'
  | 'ellipse'
  | 'polygon'
  | 'star'
  | 'line'
  | 'arrow'
  | 'pen'
  | 'text'
  | 'brush'
  | 'pencil'
  | 'eraser'
  | 'fill'
  | 'eyedropper'
  | 'gradient'
  | 'sel-rect'
  | 'sel-ellipse'
  | 'lasso'
  | 'wand'
  | 'pan'
  | 'zoom'

export type NodeKind =
  | 'artboard'
  | 'group'
  | 'layer'
  | 'shape'
  | 'path'
  | 'text'
  | 'bitmap'
  | 'stroke-set'
  | 'gradient-fill'
  | 'reference'

export interface OutlineNode {
  id: string
  kind: NodeKind
  name: string
  visible: boolean
  locked: boolean
  opacity: number
  blend: string
  modifierBadges: string[]
  children: OutlineNode[]
}

export interface HistoryEntry {
  id: number
  label: string
  undoOf: number | null
}

export type ParamValue =
  | { t: 'f64'; v: number }
  | { t: 'bool'; v: boolean }
  | { t: 'str'; v: string }
  | { t: 'color'; v: { space: string; rgba: [number, number, number, number] } }
  | { t: 'point'; v: { x: number; y: number } }
  | { t: 'expr'; v: string }
  | { t: 'ref'; v: unknown }
  | { t: 'matrix'; v: unknown }
  | { t: 'blob'; v: string }

export interface ModifierMirror {
  id: number
  kind: string
  enabled: boolean
  params: Record<string, ParamValue>
}

export interface PropsMirror {
  id: string
  kind: NodeKind
  params: Record<string, ParamValue>
  modifiers: ModifierMirror[]
}

export interface PaletteEntry {
  name: string
  color: { space: string; rgba: [number, number, number, number] }
}

export interface EditorState {
  tabs: { name: string; dirty: boolean }[]
  active: number
  tool: ToolKind
  toolParams: Record<string, ParamValue>
  fg: string
  bg: string
  view: { zoom: number; panX: number; panY: number; pixelPreview: boolean }
  outline: OutlineNode[]
  history: HistoryEntry[]
  canUndo: boolean
  canRedo: boolean
  selection: string[]
  props: PropsMirror[]
  palette: PaletteEntry[]
  variables: Record<string, ParamValue>
  hasPixelSelection: boolean
  artboards: { index: number; id: string; name: string }[]
  status: { cursorX: number; cursorY: number; cursor: string; nodesRendered: number }
  clipboardFull: boolean
}

export const EMPTY_STATE: EditorState = {
  tabs: [],
  active: 0,
  tool: 'select',
  toolParams: {},
  fg: '#1e88e5',
  bg: '#ffffff',
  view: { zoom: 1, panX: 0, panY: 0, pixelPreview: false },
  outline: [],
  history: [],
  canUndo: false,
  canRedo: false,
  selection: [],
  props: [],
  palette: [],
  variables: {},
  hasPixelSelection: false,
  artboards: [],
  status: { cursorX: 0, cursorY: 0, cursor: 'default', nodesRendered: 0 },
  clipboardFull: false,
}

/** Linear-light RGBA (core storage) → display hex for swatches. */
export function colorToHex(c: { rgba: [number, number, number, number] }): string {
  const enc = (v: number) => {
    const clamped = Math.max(0, Math.min(1, v))
    const srgb = clamped <= 0.0031308 ? clamped * 12.92 : 1.055 * Math.pow(clamped, 1 / 2.4) - 0.055
    return Math.round(srgb * 255)
      .toString(16)
      .padStart(2, '0')
  }
  return `#${enc(c.rgba[0])}${enc(c.rgba[1])}${enc(c.rgba[2])}`
}

export function paramNumber(p: ParamValue | undefined, fallback = 0): number {
  return p && p.t === 'f64' ? p.v : fallback
}

export function paramBool(p: ParamValue | undefined, fallback = false): boolean {
  return p && p.t === 'bool' ? p.v : fallback
}

export function paramString(p: ParamValue | undefined, fallback = ''): string {
  if (!p) return fallback
  if (p.t === 'str') return p.v
  if (p.t === 'expr') return `=${p.v}`
  return fallback
}
