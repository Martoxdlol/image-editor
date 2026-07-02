// Properties panel (spec §14): context-sensitive params of the selection
// plus the reorderable/toggleable modifier stack. Numeric fields accept
// expressions (prefix `=`) — spec §8.

import { core } from '@/core/bridge'
import { Input } from '@/components/ui/input'
import { Slider } from '@/components/ui/slider'
import { Switch } from '@/components/ui/switch'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import {
  colorToHex,
  paramNumber,
  paramString,
  type ParamValue,
  type PropsMirror,
} from '@/core/types'
import { ArrowDown, ArrowUp, Trash2 } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useEditorState } from '../useEditorState'

const BLEND_MODES = [
  'normal', 'multiply', 'screen', 'overlay', 'darken', 'lighten', 'color-dodge',
  'color-burn', 'hard-light', 'soft-light', 'difference', 'exclusion', 'hue',
  'saturation', 'color', 'luminosity', 'add',
]

/** Numeric field that also accepts `=expr` expressions (spec §14). */
function NumField({
  node,
  path,
  value,
  label,
}: {
  node: string
  path: string
  value: ParamValue | undefined
  label: string
}) {
  const isExpr = value?.t === 'expr'
  const display = isExpr ? `=${value.v}` : String(Math.round(paramNumber(value) * 100) / 100)
  const [text, setText] = useState(display)
  useEffect(() => setText(display), [display])

  const commit = () => {
    if (text.startsWith('=')) {
      core.cmd({ cmd: 'set-param', node, path, value: text })
    } else {
      const n = parseFloat(text)
      if (!Number.isNaN(n)) core.cmd({ cmd: 'set-param', node, path, value: n })
    }
  }

  return (
    <label className="flex items-center gap-1.5">
      <span className="num w-4 text-[10px] uppercase text-muted-foreground">{label}</span>
      <Input
        className={`num h-6 px-1.5 text-[11px] ${isExpr ? 'text-primary' : ''}`}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === 'Enter') commit()
        }}
      />
    </label>
  )
}

function ColorField({
  node,
  path,
  value,
  label,
}: {
  node: string
  path: string
  value: ParamValue | undefined
  label: string
}) {
  const hex = value?.t === 'color' ? colorToHex(value.v) : '#cccccc'
  return (
    <label className="flex items-center gap-1.5 text-[10px] uppercase text-muted-foreground">
      {label}
      <input
        type="color"
        value={hex}
        // live picking previews (no history); closing/leaving commits one txn
        onChange={(e) => core.cmd({ cmd: 'preview-param', node, path, value: e.target.value }, true)}
        onBlur={(e) => core.cmd({ cmd: 'set-param', node, path, value: e.target.value })}
        className="h-6 w-8 cursor-pointer rounded border bg-transparent"
      />
    </label>
  )
}

function NodeProps({ props }: { props: PropsMirror }) {
  const p = props.params
  const id = props.id
  const geometric = ['x', 'y', 'w', 'h', 'x2', 'y2'].filter((k) => p[k] !== undefined)
  const state = useEditorState()

  return (
    <div className="space-y-2.5 px-2 pb-2">
      <div className="flex items-center justify-between">
        <span className="text-xs font-medium">{paramString(p.name, props.kind)}</span>
        <span className="num text-[9px] text-muted-foreground">{props.kind}</span>
      </div>

      {geometric.length > 0 && (
        <div className="grid grid-cols-2 gap-1.5">
          {geometric.map((k) => (
            <NumField key={k} node={id} path={k} value={p[k]} label={k} />
          ))}
        </div>
      )}

      {/* opacity + blend on every node (spec §15) */}
      <div className="flex items-center gap-2">
        <span className="w-12 text-[10px] uppercase text-muted-foreground">Opacity</span>
        <Slider
          className="flex-1"
          min={0}
          max={1}
          step={0.01}
          value={[paramNumber(p.opacity, 1)]}
          onValueChange={([v]) =>
            core.cmd({ cmd: 'preview-param', node: id, path: 'opacity', value: v }, true)
          }
          onValueCommit={([v]) => core.cmd({ cmd: 'set-param', node: id, path: 'opacity', value: v })}
        />
        <span className="num w-8 text-right text-[10px]">
          {Math.round(paramNumber(p.opacity, 1) * 100)}%
        </span>
      </div>
      <div className="flex items-center gap-2">
        <span className="w-12 text-[10px] uppercase text-muted-foreground">Blend</span>
        <Select
          value={paramString(p.blend, 'normal')}
          onValueChange={(v) => core.cmd({ cmd: 'set-param', node: id, path: 'blend', value: v })}
        >
          <SelectTrigger size="sm" className="h-6 flex-1 text-[11px]">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {BLEND_MODES.map((b) => (
              <SelectItem key={b} value={b} className="text-xs">
                {b}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      {/* fills & strokes */}
      <div className="flex flex-wrap items-center gap-3">
        {p['fill-color'] !== undefined && (
          <ColorField node={id} path="fill-color" value={p['fill-color']} label="Fill" />
        )}
        {p['stroke-color'] !== undefined && (
          <ColorField node={id} path="stroke-color" value={p['stroke-color']} label="Stroke" />
        )}
        {p['stroke-width'] !== undefined && (
          <NumField node={id} path="stroke-width" value={p['stroke-width']} label="W" />
        )}
        {p['from-color'] !== undefined && (
          <ColorField node={id} path="from-color" value={p['from-color']} label="From" />
        )}
        {p['to-color'] !== undefined && (
          <ColorField node={id} path="to-color" value={p['to-color']} label="To" />
        )}
        {p['bg-color'] !== undefined && (
          <ColorField node={id} path="bg-color" value={p['bg-color']} label="BG" />
        )}
      </div>

      {/* shape-specific */}
      {props.kind === 'shape' && (
        <div className="grid grid-cols-2 gap-1.5">
          {p.radius !== undefined && <NumField node={id} path="radius" value={p.radius} label="r" />}
          {p.sides !== undefined && <NumField node={id} path="sides" value={p.sides} label="n" />}
        </div>
      )}

      {/* text editing (spec §7 scoped: panel-based) */}
      {props.kind === 'text' && (
        <div className="space-y-1.5">
          <textarea
            className="min-h-16 w-full rounded border bg-input/30 p-1.5 text-xs"
            value={paramString(p.text)}
            // keystrokes preview live; one txn commits when leaving the field
            onChange={(e) => core.cmd({ cmd: 'preview-param', node: id, path: 'text', value: e.target.value }, true)}
            onBlur={(e) => core.cmd({ cmd: 'set-param', node: id, path: 'text', value: e.target.value })}
          />
          <div className="grid grid-cols-2 gap-1.5">
            <NumField node={id} path="font-size" value={p['font-size']} label="Sz" />
            <label className="flex items-center gap-1.5">
              <span className="text-[10px] uppercase text-muted-foreground">Align</span>
              <Select
                value={paramString(p.align, 'left')}
                onValueChange={(v) => core.cmd({ cmd: 'set-param', node: id, path: 'align', value: v })}
              >
                <SelectTrigger size="sm" className="h-6 flex-1 text-[11px]">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {['left', 'center', 'right'].map((a) => (
                    <SelectItem key={a} value={a} className="text-xs">
                      {a}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>
          </div>
        </div>
      )}

      {/* artboard background */}
      {props.kind === 'artboard' && (
        <label className="flex items-center gap-1.5">
          <span className="text-[10px] uppercase text-muted-foreground">Background</span>
          <Select
            value={paramString(p.background, 'color')}
            onValueChange={(v) => core.cmd({ cmd: 'set-param', node: id, path: 'background', value: v })}
          >
            <SelectTrigger size="sm" className="h-6 flex-1 text-[11px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {['color', 'transparent', 'checker'].map((b) => (
                <SelectItem key={b} value={b} className="text-xs">
                  {b}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </label>
      )}

      {/* modifier stack (spec §2.2: ordered, toggleable, reorderable) */}
      {props.modifiers.length > 0 && (
        <div className="space-y-1">
          <div className="panel-title pt-1">Modifiers</div>
          {props.modifiers.map((m, mi) => (
            <div key={m.id} className="rounded border bg-card/60 p-1.5">
              <div className="flex items-center gap-1.5">
                <Switch
                  checked={m.enabled}
                  onCheckedChange={(v) =>
                    core.cmd({ cmd: 'set-param', node: id, path: `mod.${m.id}.enabled`, value: v })
                  }
                  className="scale-75"
                />
                <span className="flex-1 truncate text-[11px]">{m.kind}</span>
                <button
                  className="rounded p-0.5 text-muted-foreground hover:text-foreground disabled:opacity-30"
                  disabled={mi === 0}
                  onClick={() => core.cmd({ cmd: 'reorder-modifier', node: id, id: m.id, index: mi - 1 })}
                >
                  <ArrowUp size={11} />
                </button>
                <button
                  className="rounded p-0.5 text-muted-foreground hover:text-foreground disabled:opacity-30"
                  disabled={mi === props.modifiers.length - 1}
                  onClick={() => core.cmd({ cmd: 'reorder-modifier', node: id, id: m.id, index: mi + 1 })}
                >
                  <ArrowDown size={11} />
                </button>
                <button
                  className="rounded p-0.5 text-muted-foreground hover:text-destructive"
                  onClick={() => core.cmd({ cmd: 'remove-modifier', node: id, id: m.id })}
                >
                  <Trash2 size={11} />
                </button>
              </div>
              <div className="mt-1 grid grid-cols-2 gap-1.5">
                {Object.entries(m.params)
                  .filter(([, v]) => v.t === 'f64' || v.t === 'expr')
                  .map(([k, v]) => (
                    <NumField key={k} node={id} path={`mod.${m.id}.${k}`} value={v} label={k.slice(0, 4)} />
                  ))}
              </div>
            </div>
          ))}
        </div>
      )}
      {state.selection.length > 1 && (
        <div className="text-[10px] text-muted-foreground">{state.selection.length} selected</div>
      )}
    </div>
  )
}

export default function PropertiesPanel() {
  const state = useEditorState()
  return (
    <div className="flex min-h-0 flex-1 flex-col border-t">
      <div className="panel-title px-2 py-1.5">Properties</div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {state.props.length === 0 ? (
          <div className="px-2 text-[11px] text-muted-foreground">
            Nothing selected. Numeric fields accept <span className="num text-primary">=expressions</span> like{' '}
            <span className="num">=$gridSize * 2</span>.
          </div>
        ) : (
          state.props.map((p) => <NodeProps key={p.id} props={p} />)
        )}
      </div>
    </div>
  )
}
