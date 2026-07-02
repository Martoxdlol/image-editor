// System-clipboard interop (spec §10.6/§10.7, clipboard v1 scope §16.6):
// every copy writes the internal fragment (core-side) AND a PNG flavor to
// the OS clipboard; paste prefers the internal fragment, falling back to
// OS images (import-on-paste).

import { core } from '@/core/bridge'

export async function copySelection(cut = false): Promise<void> {
  // internal, full fidelity
  core.cmd({ cmd: cut ? 'cut' : 'copy' })
  // system PNG flavor, best effort (browser may deny without user gesture)
  try {
    const bytes = await core.copyAsPng()
    if (bytes.byteLength > 0 && navigator.clipboard && 'write' in navigator.clipboard) {
      const item = new ClipboardItem({ 'image/png': new Blob([bytes], { type: 'image/png' }) })
      await navigator.clipboard.write([item])
    }
  } catch {
    // PNG flavor is best-effort; internal clipboard already succeeded
  }
}

export async function pasteFromClipboard(inPlace = false): Promise<void> {
  // internal fragment wins (highest fidelity, spec §10.1)
  const state = core.getState()
  if (state.clipboardFull) {
    core.cmd({ cmd: 'paste', in_place: inPlace })
    return
  }
  // fall back to OS clipboard images → import-on-paste (spec §10.7)
  try {
    const items = await navigator.clipboard.read()
    for (const item of items) {
      const type = item.types.find((t) => t.startsWith('image/'))
      if (type) {
        const blob = await item.getType(type)
        const buf = await blob.arrayBuffer()
        core.importImage(buf, 'Pasted image')
        return
      }
    }
  } catch {
    // no readable clipboard or permission denied — nothing to paste
  }
}
