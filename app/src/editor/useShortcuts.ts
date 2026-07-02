// Global keyboard shortcuts (spec §14). Key events are interpreted here
// into semantic commands; tool-level keys forward to the core.

import { useEffect } from 'react'
import { core } from '@/core/bridge'
import { copySelection, pasteFromClipboard } from './clipboard'
import { TOOL_SHORTCUTS } from './Toolbar'

export function useShortcuts(handlers: { newDoc: () => void; exportDlg: () => void }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement
      const editing =
        target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable
      const cmd = e.metaKey || e.ctrlKey

      if (editing && !cmd) return

      if (cmd) {
        switch (e.key.toLowerCase()) {
          case 'z':
            e.preventDefault()
            core.cmd({ cmd: e.shiftKey ? 'redo' : 'undo' })
            return
          case 'y':
            e.preventDefault()
            core.cmd({ cmd: 'redo' })
            return
          case 'c':
            if (editing) return
            e.preventDefault()
            copySelection(false)
            return
          case 'x':
            if (editing) return
            e.preventDefault()
            copySelection(true)
            return
          case 'v':
            if (editing) return
            e.preventDefault()
            pasteFromClipboard(e.shiftKey)
            return
          case 'd':
            e.preventDefault()
            core.cmd({ cmd: 'duplicate-selection' })
            return
          case 'g':
            e.preventDefault()
            core.cmd({ cmd: e.shiftKey ? 'ungroup-selection' : 'group-selection' })
            return
          case 'a':
            if (editing) return
            e.preventDefault()
            core.cmd({ cmd: e.shiftKey ? 'clear-pixel-selection' : 'select-all' })
            return
          case 'n':
            e.preventDefault()
            handlers.newDoc()
            return
          case 'e':
            if (e.shiftKey) {
              e.preventDefault()
              handlers.exportDlg()
            }
            return
          case '=':
          case '+':
            e.preventDefault()
            core.cmd({ cmd: 'zoom-by', factor: 1.25, cx: 400, cy: 300 })
            return
          case '-':
            e.preventDefault()
            core.cmd({ cmd: 'zoom-by', factor: 0.8, cx: 400, cy: 300 })
            return
          case '1':
            e.preventDefault()
            core.cmd({ cmd: 'set-view', zoom: Math.min(window.devicePixelRatio || 1, 2) })
            return
          case 'tab':
            return
        }
        return
      }

      if (editing) return

      // tool shortcuts
      const tool = TOOL_SHORTCUTS[e.key.toLowerCase()]
      if (tool && !e.repeat) {
        core.cmd({ cmd: 'set-tool', tool })
        return
      }
      if (e.key === '1' && e.shiftKey) {
        core.cmd({ cmd: 'fit-view' })
        return
      }
      // semantic keys handled by the core (nudge, delete, escape, enter)
      if (
        ['Escape', 'Enter', 'Delete', 'Backspace', 'ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(
          e.key,
        )
      ) {
        e.preventDefault()
        core.cmd({
          cmd: 'key',
          key: e.key,
          mods: { shift: e.shiftKey, alt: e.altKey, ctrl: e.ctrlKey, meta: e.metaKey },
        })
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [handlers])
}
