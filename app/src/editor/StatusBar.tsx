// Status bar (spec §14): cursor pos, zoom, render mode, perf counter.

import { core } from '@/core/bridge'
import { useEditorState } from './useEditorState'
import { DPR } from './CanvasView'

export default function StatusBar() {
  const state = useEditorState()
  const zoomPct = Math.round((state.view.zoom / DPR) * 100)
  return (
    <div className="flex h-6 items-center gap-4 border-t bg-sidebar px-3 text-[10px] text-muted-foreground">
      <span className="num w-28">
        {Math.round(state.status.cursorX)}, {Math.round(state.status.cursorY)} px
      </span>
      <button
        className="num rounded px-1 hover:bg-accent hover:text-foreground"
        onClick={() => core.cmd({ cmd: 'fit-view' })}
        title="Click to fit"
      >
        {zoomPct}%
      </button>
      <button
        className="rounded px-1 hover:bg-accent hover:text-foreground"
        onClick={() => core.cmd({ cmd: 'set-pixel-preview', on: !state.view.pixelPreview })}
      >
        {state.view.pixelPreview ? 'pixel preview' : 'vector'}
      </button>
      {state.hasPixelSelection && <span className="text-primary">selection active</span>}
      <span className="num ml-auto" title="nodes rendered last frame (perf HUD)">
        {state.status.nodesRendered} nodes
      </span>
      <span className="num">{state.selection.length ? `${state.selection.length} sel` : ''}</span>
    </div>
  )
}
