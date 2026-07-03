// Main-thread bridge to the core worker (spec §12.3). Exposes a command
// API and a React-consumable store of the latest state mirror.

import { EMPTY_STATE, type EditorState } from './types'

type Listener = () => void

class CoreBridge {
  private worker: Worker | null = null
  private state: EditorState = EMPTY_STATE
  private listeners = new Set<Listener>()
  private pending = new Map<number, (bytes: ArrayBuffer) => void>()
  private nextId = 1
  ready = false
  lastError: string | null = null

  start(canvas: HTMLCanvasElement, width: number, height: number): void {
    if (this.worker) return
    this.worker = new Worker(new URL('./worker.ts', import.meta.url), { type: 'module' })
    this.worker.onmessage = (e) => this.onMessage(e)
    const offscreen = canvas.transferControlToOffscreen()
    this.worker.postMessage({ type: 'init', canvas: offscreen, width, height }, [offscreen])
  }

  private onMessage(e: MessageEvent) {
    const msg = e.data
    switch (msg.type) {
      case 'ready':
        this.ready = true
        this.emit()
        break
      case 'state':
        this.state = JSON.parse(msg.state)
        this.emit()
        break
      case 'error':
        this.lastError = msg.error
        console.error('[core]', msg.error)
        this.emit()
        break
      case 'result': {
        const cb = this.pending.get(msg.id)
        if (cb) {
          this.pending.delete(msg.id)
          cb(msg.bytes)
        }
        break
      }
    }
  }

  private emit() {
    for (const l of this.listeners) l()
  }

  // React store interface
  subscribe = (l: Listener): (() => void) => {
    this.listeners.add(l)
    return () => this.listeners.delete(l)
  }

  getState = (): EditorState => this.state

  /** Send a command. `hot` marks high-frequency events (pointer moves). */
  cmd(command: Record<string, unknown>, hot = false): void {
    this.worker?.postMessage({ type: 'cmd', json: JSON.stringify(command), hot })
  }

  resize(width: number, height: number): void {
    this.worker?.postMessage({ type: 'resize', width, height })
  }

  importImage(
    bytes: ArrayBuffer,
    name: string,
    opts: { scale?: number; newDoc?: boolean } = {},
  ): void {
    this.worker?.postMessage(
      { type: 'import-image', bytes, name, scale: opts.scale ?? 1, newDoc: opts.newDoc ?? false },
      [bytes],
    )
  }

  openMyed(bytes: ArrayBuffer, name: string): void {
    this.worker?.postMessage({ type: 'open-myed', bytes, name }, [bytes])
  }

  private request(msg: Record<string, unknown>): Promise<ArrayBuffer> {
    const id = this.nextId++
    return new Promise((resolve) => {
      this.pending.set(id, resolve)
      this.worker?.postMessage({ ...msg, id })
    })
  }

  exportArtboard(
    artboard: number,
    scale: number,
    format: string,
    opts: { background?: boolean; quality?: number } = {},
  ): Promise<ArrayBuffer> {
    return this.request({
      type: 'export',
      artboard,
      scale,
      format,
      background: opts.background ?? true,
      quality: opts.quality ?? 90,
    })
  }

  copyAsPng(): Promise<ArrayBuffer> {
    return this.request({ type: 'copy-png' })
  }

  saveMyed(): Promise<ArrayBuffer> {
    return this.request({ type: 'save-myed' })
  }
}

export const core = new CoreBridge()

export function downloadBytes(bytes: ArrayBuffer, name: string, mime: string): void {
  const blob = new Blob([bytes], { type: mime })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = name
  a.click()
  setTimeout(() => URL.revokeObjectURL(url), 5000)
}
