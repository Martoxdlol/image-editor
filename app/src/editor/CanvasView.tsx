// The viewport: an OffscreenCanvas rendered by the core worker. This
// component only forwards input (spec §12.3) and manages sizing.

import { useEffect, useRef } from 'react'
import { core } from '@/core/bridge'
import { useEditorState } from './useEditorState'

const DPR = Math.min(window.devicePixelRatio || 1, 2)

const CURSOR_BY_TOOL: Record<string, string> = {
  select: 'default',
  pan: 'grab',
  zoom: 'zoom-in',
  text: 'text',
  eyedropper: 'crosshair',
  brush: 'none',
  pencil: 'none',
  eraser: 'none',
  fill: 'crosshair',
  wand: 'crosshair',
  'sel-rect': 'crosshair',
  'sel-ellipse': 'crosshair',
  lasso: 'crosshair',
  pen: 'crosshair',
  rect: 'crosshair',
  ellipse: 'crosshair',
  polygon: 'crosshair',
  star: 'crosshair',
  line: 'crosshair',
  arrow: 'crosshair',
  gradient: 'crosshair',
}

function mods(e: { shiftKey: boolean; altKey: boolean; ctrlKey: boolean; metaKey: boolean }) {
  return { shift: e.shiftKey, alt: e.altKey, ctrl: e.ctrlKey, meta: e.metaKey }
}

export default function CanvasView() {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const holderRef = useRef<HTMLDivElement>(null)
  const startedRef = useRef(false)
  const state = useEditorState()

  useEffect(() => {
    const canvas = canvasRef.current
    const holder = holderRef.current
    if (!canvas || !holder || startedRef.current) return
    startedRef.current = true
    const rect = holder.getBoundingClientRect()
    const w = Math.max(64, Math.round(rect.width * DPR))
    const h = Math.max(64, Math.round(rect.height * DPR))
    canvas.style.width = `${rect.width}px`
    canvas.style.height = `${rect.height}px`
    core.start(canvas, w, h)

    const ro = new ResizeObserver(() => {
      const r = holder.getBoundingClientRect()
      canvas.style.width = `${r.width}px`
      canvas.style.height = `${r.height}px`
      core.resize(Math.max(64, Math.round(r.width * DPR)), Math.max(64, Math.round(r.height * DPR)))
    })
    ro.observe(holder)
    return () => ro.disconnect()
  }, [])

  // pointer events → core (coalesced moves, pressure — spec §12.3)
  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return

    const pos = (e: PointerEvent) => {
      const r = canvas.getBoundingClientRect()
      return { x: (e.clientX - r.left) * DPR, y: (e.clientY - r.top) * DPR }
    }

    const down = (e: PointerEvent) => {
      if (e.button !== 0 && e.button !== 1) return
      canvas.setPointerCapture(e.pointerId)
      const { x, y } = pos(e)
      // middle mouse = temporary pan
      if (e.button === 1) {
        core.cmd({ cmd: 'set-tool', tool: 'pan' })
      }
      core.cmd({
        cmd: 'pointer',
        kind: 'down',
        x,
        y,
        pressure: e.pressure || 1,
        button: e.button,
        mods: mods(e),
      })
      e.preventDefault()
    }
    const move = (e: PointerEvent) => {
      const events = 'getCoalescedEvents' in e ? e.getCoalescedEvents() : [e]
      for (const ce of events.length ? events : [e]) {
        const { x, y } = pos(ce as PointerEvent)
        core.cmd(
          {
            cmd: 'pointer',
            kind: 'move',
            x,
            y,
            pressure: (ce as PointerEvent).pressure || 1,
            mods: mods(e),
          },
          true,
        )
      }
    }
    const up = (e: PointerEvent) => {
      const { x, y } = pos(e)
      core.cmd({ cmd: 'pointer', kind: 'up', x, y, pressure: e.pressure || 1, mods: mods(e) })
    }
    const dbl = (e: MouseEvent) => {
      const r = canvas.getBoundingClientRect()
      core.cmd({
        cmd: 'pointer',
        kind: 'double-click',
        x: (e.clientX - r.left) * DPR,
        y: (e.clientY - r.top) * DPR,
        mods: mods(e),
      })
    }
    const wheel = (e: WheelEvent) => {
      e.preventDefault()
      const r = canvas.getBoundingClientRect()
      const cx = (e.clientX - r.left) * DPR
      const cy = (e.clientY - r.top) * DPR
      if (e.ctrlKey || e.metaKey) {
        const factor = Math.exp(-e.deltaY * 0.0022)
        core.cmd({ cmd: 'zoom-by', factor, cx, cy }, true)
      } else {
        const s = core.getState()
        const z = s.view.zoom || 1
        core.cmd(
          {
            cmd: 'set-view',
            pan_x: s.view.panX + (e.shiftKey ? e.deltaY : e.deltaX) / z,
            pan_y: s.view.panY + (e.shiftKey ? 0 : e.deltaY) / z,
          },
          true,
        )
      }
    }

    canvas.addEventListener('pointerdown', down)
    canvas.addEventListener('pointermove', move)
    canvas.addEventListener('pointerup', up)
    canvas.addEventListener('dblclick', dbl)
    canvas.addEventListener('wheel', wheel, { passive: false })
    return () => {
      canvas.removeEventListener('pointerdown', down)
      canvas.removeEventListener('pointermove', move)
      canvas.removeEventListener('pointerup', up)
      canvas.removeEventListener('dblclick', dbl)
      canvas.removeEventListener('wheel', wheel)
    }
  }, [])

  // drag & drop import (spec §9)
  useEffect(() => {
    const holder = holderRef.current
    if (!holder) return
    const over = (e: DragEvent) => e.preventDefault()
    const drop = async (e: DragEvent) => {
      e.preventDefault()
      const files = Array.from(e.dataTransfer?.files ?? [])
      for (const f of files) {
        const buf = await f.arrayBuffer()
        if (f.name.endsWith('.myed')) core.openMyed(buf, f.name)
        else core.importImage(buf, f.name)
      }
    }
    holder.addEventListener('dragover', over)
    holder.addEventListener('drop', drop)
    return () => {
      holder.removeEventListener('dragover', over)
      holder.removeEventListener('drop', drop)
    }
  }, [])

  return (
    <div ref={holderRef} className="relative flex-1 overflow-hidden bg-[#222226]">
      <canvas
        ref={canvasRef}
        className="absolute left-0 top-0 touch-none"
        style={{ cursor: CURSOR_BY_TOOL[state.tool] ?? 'default' }}
      />
    </div>
  )
}

export { DPR }
