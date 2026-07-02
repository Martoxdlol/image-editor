// Color panel (spec §14): fg/bg colors, document palette (named colors,
// live-propagating refs §6.7), and document variables (§6.7/§8).

import { core } from '@/core/bridge'
import { Input } from '@/components/ui/input'
import { colorToHex, paramNumber } from '@/core/types'
import { Plus, X } from 'lucide-react'
import { useState } from 'react'
import { useEditorState } from '../useEditorState'

export default function ColorPanel() {
  const state = useEditorState()
  const [varName, setVarName] = useState('')
  const [varValue, setVarValue] = useState('')

  const addPaletteEntry = () => {
    const name = window.prompt('Palette color name', `color-${state.palette.length + 1}`)
    if (name) core.cmd({ cmd: 'set-palette', name, color: state.fg })
  }

  const addVariable = () => {
    if (!varName) return
    const n = parseFloat(varValue)
    core.cmd({
      cmd: 'set-variable',
      name: varName,
      value: Number.isNaN(n) ? varValue : n,
    })
    setVarName('')
    setVarValue('')
  }

  return (
    <div className="shrink-0 border-t">
      <div className="panel-title px-2 py-1.5">Color</div>
      <div className="flex items-center gap-3 px-2 pb-2">
        <div className="relative h-10 w-10">
          <input
            type="color"
            value={state.bg}
            onChange={(e) => core.cmd({ cmd: 'set-bg', color: e.target.value })}
            title="Background color"
            className="absolute bottom-0 right-0 h-6 w-6 cursor-pointer rounded border-2 border-background"
          />
          <input
            type="color"
            value={state.fg}
            onChange={(e) => core.cmd({ cmd: 'set-fg', color: e.target.value })}
            title="Foreground color"
            className="absolute left-0 top-0 h-7 w-7 cursor-pointer rounded border-2 border-background shadow"
          />
        </div>
        <div className="num text-[10px] leading-4 text-muted-foreground">
          <div>
            fg <span className="text-foreground">{state.fg}</span>
          </div>
          <div>
            bg <span className="text-foreground">{state.bg}</span>
          </div>
        </div>
      </div>

      <div className="panel-title flex items-center justify-between px-2 py-1">
        <span>Palette</span>
        <button onClick={addPaletteEntry} className="rounded p-0.5 text-muted-foreground hover:text-foreground">
          <Plus size={11} />
        </button>
      </div>
      <div className="flex flex-wrap gap-1 px-2 pb-2">
        {state.palette.length === 0 && (
          <span className="text-[10px] text-muted-foreground">No named colors yet</span>
        )}
        {state.palette.map((e) => (
          <div key={e.name} className="group relative" title={e.name}>
            <button
              className="h-5 w-5 rounded-sm border"
              style={{ background: colorToHex(e.color) }}
              onClick={() => core.cmd({ cmd: 'set-fg', color: colorToHex(e.color) })}
            />
            <button
              className="absolute -right-1 -top-1 hidden rounded-full bg-destructive p-px group-hover:block"
              onClick={() => core.cmd({ cmd: 'set-palette', name: e.name, color: null })}
            >
              <X size={8} />
            </button>
          </div>
        ))}
      </div>

      <div className="panel-title px-2 py-1">Variables</div>
      <div className="space-y-1 px-2 pb-2">
        {Object.entries(state.variables).map(([name, v]) => (
          <div key={name} className="flex items-center gap-1.5">
            <span className="num flex-1 truncate text-[11px] text-primary">${name}</span>
            <Input
              className="num h-5 w-16 px-1 text-[11px]"
              defaultValue={String(paramNumber(v))}
              key={`${name}=${paramNumber(v)}`}
              onBlur={(e) => {
                const n = parseFloat(e.target.value)
                if (!Number.isNaN(n)) core.cmd({ cmd: 'set-variable', name, value: n })
              }}
              onKeyDown={(e) => {
                if (e.key === 'Enter') (e.target as HTMLInputElement).blur()
              }}
            />
            <button
              className="text-muted-foreground hover:text-destructive"
              onClick={() => core.cmd({ cmd: 'set-variable', name, value: null })}
            >
              <X size={10} />
            </button>
          </div>
        ))}
        <div className="flex items-center gap-1">
          <Input
            className="num h-5 flex-1 px-1 text-[11px]"
            placeholder="name"
            value={varName}
            onChange={(e) => setVarName(e.target.value)}
          />
          <Input
            className="num h-5 w-14 px-1 text-[11px]"
            placeholder="8"
            value={varValue}
            onChange={(e) => setVarValue(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') addVariable()
            }}
          />
          <button onClick={addVariable} className="text-muted-foreground hover:text-foreground">
            <Plus size={11} />
          </button>
        </div>
      </div>
    </div>
  )
}
