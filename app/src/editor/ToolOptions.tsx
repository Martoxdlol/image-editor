// Tool options bar (spec §14): active tool params, auto-rendered from the
// param schema (spec §3.4).

import { core } from '@/core/bridge'
import { Slider } from '@/components/ui/slider'
import { Switch } from '@/components/ui/switch'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { paramBool, paramNumber, paramString, type ToolKind } from '@/core/types'
import { useEditorState } from './useEditorState'

interface NumOpt {
  kind: 'num'
  key: string
  label: string
  min: number
  max: number
  step?: number
  def: number
}
interface BoolOpt {
  kind: 'bool'
  key: string
  label: string
  def: boolean
}
interface EnumOpt {
  kind: 'enum'
  key: string
  label: string
  values: string[]
  def: string
}
type Opt = NumOpt | BoolOpt | EnumOpt

const OPTIONS: Partial<Record<ToolKind, Opt[]>> = {
  brush: [
    { kind: 'num', key: 'brush.size', label: 'Size', min: 1, max: 200, def: 16 },
    { kind: 'num', key: 'brush.hardness', label: 'Hardness', min: 0, max: 1, step: 0.01, def: 0.8 },
    { kind: 'num', key: 'brush.opacity', label: 'Opacity', min: 0, max: 1, step: 0.01, def: 1 },
    { kind: 'num', key: 'brush.flow', label: 'Flow', min: 0.05, max: 1, step: 0.01, def: 1 },
    { kind: 'bool', key: 'brush.as-strokes', label: 'Paint as strokes', def: false },
  ],
  pencil: [{ kind: 'num', key: 'pencil.size', label: 'Size', min: 1, max: 64, def: 1 }],
  eraser: [
    { kind: 'num', key: 'eraser.size', label: 'Size', min: 1, max: 200, def: 24 },
    { kind: 'num', key: 'eraser.hardness', label: 'Hardness', min: 0, max: 1, step: 0.01, def: 1 },
  ],
  fill: [
    { kind: 'num', key: 'fill.tolerance', label: 'Tolerance', min: 0, max: 1, step: 0.01, def: 0.1 },
    { kind: 'bool', key: 'fill.contiguous', label: 'Contiguous', def: true },
  ],
  wand: [
    { kind: 'num', key: 'wand.tolerance', label: 'Tolerance', min: 0, max: 1, step: 0.01, def: 0.15 },
    { kind: 'bool', key: 'wand.contiguous', label: 'Contiguous', def: true },
  ],
  rect: [
    { kind: 'num', key: 'shape.radius', label: 'Corner radius', min: 0, max: 100, def: 0 },
    { kind: 'num', key: 'shape.stroke-width', label: 'Stroke', min: 0, max: 40, def: 0 },
  ],
  ellipse: [{ kind: 'num', key: 'shape.stroke-width', label: 'Stroke', min: 0, max: 40, def: 0 }],
  polygon: [
    { kind: 'num', key: 'shape.sides', label: 'Sides', min: 3, max: 16, step: 1, def: 6 },
    { kind: 'num', key: 'shape.stroke-width', label: 'Stroke', min: 0, max: 40, def: 0 },
  ],
  star: [
    { kind: 'num', key: 'shape.points', label: 'Points', min: 3, max: 16, step: 1, def: 5 },
    { kind: 'num', key: 'shape.stroke-width', label: 'Stroke', min: 0, max: 40, def: 0 },
  ],
  line: [{ kind: 'num', key: 'shape.stroke-width', label: 'Width', min: 1, max: 40, def: 2 }],
  arrow: [{ kind: 'num', key: 'shape.stroke-width', label: 'Width', min: 1, max: 40, def: 2 }],
  pen: [{ kind: 'num', key: 'pen.stroke-width', label: 'Stroke', min: 0, max: 40, def: 2 }],
  text: [{ kind: 'num', key: 'text.size', label: 'Size', min: 6, max: 240, def: 24 }],
  gradient: [
    { kind: 'enum', key: 'gradient.kind', label: 'Type', values: ['linear', 'radial', 'reflected'], def: 'linear' },
  ],
  'sel-rect': [
    { kind: 'num', key: 'sel.feather', label: 'Feather', min: 0, max: 50, step: 1, def: 0 },
  ],
  'sel-ellipse': [
    { kind: 'num', key: 'sel.feather', label: 'Feather', min: 0, max: 50, step: 1, def: 0 },
  ],
}

const HINTS: Partial<Record<ToolKind, string>> = {
  select: 'Drag to move · handles to resize · ⇧ multi-select · double-click to enter groups',
  pen: 'Click to add points · click the first point to close · Enter finishes · Esc cancels',
  wand: '⇧ add · ⌥ subtract',
  'sel-rect': '⇧ add · ⌥ subtract · ⇧⌥ intersect',
  'sel-ellipse': '⇧ add · ⌥ subtract',
  lasso: 'Drag a freehand region',
  zoom: 'Click to zoom in · ⌥-click out',
  eyedropper: 'Click to pick foreground · ⌥ background',
  fill: 'Click a bitmap to flood fill · click a shape to recolor',
  gradient: 'Drag to place a gradient fill region',
}

export default function ToolOptions() {
  const state = useEditorState()
  const opts = OPTIONS[state.tool] ?? []

  return (
    <div className="flex h-9 items-center gap-4 border-b bg-background px-3">
      <span className="panel-title w-20 shrink-0">{state.tool.replace('-', ' ')}</span>
      {opts.map((opt) => {
        if (opt.kind === 'num') {
          const val = paramNumber(state.toolParams[opt.key], opt.def)
          return (
            <label key={opt.key} className="flex items-center gap-2 text-xs text-muted-foreground">
              {opt.label}
              <Slider
                className="w-24"
                min={opt.min}
                max={opt.max}
                step={opt.step ?? (opt.max - opt.min) / 100}
                value={[val]}
                onValueChange={([v]) =>
                  core.cmd({ cmd: 'set-tool-param', key: opt.key, value: { t: 'f64', v } }, true)
                }
              />
              <span className="num w-9 text-right text-[11px] text-foreground">
                {opt.max <= 1 ? `${Math.round(val * 100)}%` : Math.round(val)}
              </span>
            </label>
          )
        }
        if (opt.kind === 'bool') {
          const val = paramBool(state.toolParams[opt.key], opt.def)
          return (
            <label key={opt.key} className="flex items-center gap-2 text-xs text-muted-foreground">
              {opt.label}
              <Switch
                checked={val}
                onCheckedChange={(v) =>
                  core.cmd({ cmd: 'set-tool-param', key: opt.key, value: { t: 'bool', v } })
                }
              />
            </label>
          )
        }
        const val = paramString(state.toolParams[opt.key], opt.def)
        return (
          <label key={opt.key} className="flex items-center gap-2 text-xs text-muted-foreground">
            {opt.label}
            <Select
              value={val}
              onValueChange={(v) =>
                core.cmd({ cmd: 'set-tool-param', key: opt.key, value: { t: 'str', v } })
              }
            >
              <SelectTrigger size="sm" className="h-6 w-28 text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {opt.values.map((v) => (
                  <SelectItem key={v} value={v} className="text-xs">
                    {v}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </label>
        )
      })}
      <span className="ml-auto truncate text-[11px] text-muted-foreground/70">
        {HINTS[state.tool] ?? ''}
      </span>
    </div>
  )
}
