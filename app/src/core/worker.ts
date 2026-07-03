/// <reference lib="webworker" />
// Core worker (spec §12.3): owns the Rust session and the OffscreenCanvas.
// React never touches document data — commands in, state mirrors out.

import init, { EditorSession } from '../wasm-pkg/ed_wasm'

let session: EditorSession | null = null
let wasmMemory: WebAssembly.Memory | null = null
let canvas: OffscreenCanvas | null = null
let ctx: OffscreenCanvasRenderingContext2D | null = null
let width = 800
let height = 600
let stateTimer: ReturnType<typeof setTimeout> | null = null

function postState() {
  if (!session) return
  const state = session.state()
  ;(self as unknown as Worker).postMessage({ type: 'state', state })
}

function scheduleState() {
  if (stateTimer) return
  stateTimer = setTimeout(() => {
    stateTimer = null
    postState()
  }, 30)
}

function renderLoop() {
  if (session && ctx && canvas) {
    if (session.needs_frame()) {
      const phase = (performance.now() / 60) % 8
      session.render(width, height, phase)
      const len = session.frame_len()
      if (len === width * height * 4 && wasmMemory) {
        const view = new Uint8ClampedArray(wasmMemory.buffer, session.frame_ptr(), len)
        // copy out of wasm memory (buffer may grow between frames)
        const data = new ImageData(new Uint8ClampedArray(view), width, height)
        ctx.putImageData(data, 0, 0)
      }
    }
  }
  requestAnimationFrame(renderLoop)
}

self.onmessage = async (e: MessageEvent) => {
  const msg = e.data
  switch (msg.type) {
    case 'init': {
      const out = await init()
      wasmMemory = out.memory
      session = new EditorSession()
      canvas = msg.canvas as OffscreenCanvas
      width = msg.width
      height = msg.height
      canvas.width = width
      canvas.height = height
      ctx = canvas.getContext('2d')
      session.command(JSON.stringify({ cmd: 'resize', width, height }))
      session.command(JSON.stringify({ cmd: 'fit-view' }))
      postState()
      ;(self as unknown as Worker).postMessage({ type: 'ready' })
      renderLoop()
      break
    }
    case 'resize': {
      width = Math.max(1, msg.width)
      height = Math.max(1, msg.height)
      if (canvas) {
        canvas.width = width
        canvas.height = height
      }
      session?.command(JSON.stringify({ cmd: 'resize', width, height }))
      break
    }
    case 'cmd': {
      if (!session) return
      const res = JSON.parse(session.command(msg.json))
      if (res.ok === false) {
        ;(self as unknown as Worker).postMessage({ type: 'error', error: res.error })
      }
      // pointer moves are hot: throttle state updates for them
      if (msg.hot) scheduleState()
      else postState()
      break
    }
    case 'import-image': {
      if (!session) return
      const r = JSON.parse(
        session.import_image(new Uint8Array(msg.bytes), msg.name, msg.scale ?? 1, msg.newDoc ?? false),
      )
      if (r.ok === false) {
        ;(self as unknown as Worker).postMessage({ type: 'error', error: r.error })
      }
      postState()
      break
    }
    case 'open-myed': {
      if (!session) return
      const r = JSON.parse(session.open_myed(new Uint8Array(msg.bytes), msg.name))
      if (r.ok === false) {
        ;(self as unknown as Worker).postMessage({ type: 'error', error: r.error })
      }
      session.command(JSON.stringify({ cmd: 'fit-view' }))
      postState()
      break
    }
    case 'export': {
      if (!session) return
      const bytes = session.export_artboard(
        msg.artboard ?? 0,
        msg.scale ?? 1,
        msg.format ?? 'png',
        msg.background ?? true,
        msg.quality ?? 90,
      )
      ;(self as unknown as Worker).postMessage(
        { type: 'result', id: msg.id, bytes: bytes.buffer, name: msg.name },
        { transfer: [bytes.buffer] },
      )
      break
    }
    case 'copy-png': {
      if (!session) return
      const bytes = session.copy_as_png()
      ;(self as unknown as Worker).postMessage(
        { type: 'result', id: msg.id, bytes: bytes.buffer },
        { transfer: [bytes.buffer] },
      )
      break
    }
    case 'save-myed': {
      if (!session) return
      const bytes = session.save_myed()
      ;(self as unknown as Worker).postMessage(
        { type: 'result', id: msg.id, bytes: bytes.buffer },
        { transfer: [bytes.buffer] },
      )
      postState()
      break
    }
  }
}
