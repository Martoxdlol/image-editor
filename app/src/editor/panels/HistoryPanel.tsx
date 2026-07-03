// History panel (spec §14): txn-level entries from the append-only op log
// (undo entries appear as new rows — history is never rewritten, §3.3).
// Right-click a txn → Revert to here (jump-to).

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from '@/components/ui/context-menu'
import { core } from '@/core/bridge'
import { cn } from '@/lib/utils'
import { Undo2 } from 'lucide-react'
import { useEffect, useRef } from 'react'
import { useEditorState } from '../useEditorState'
import Section from './Section'

export default function HistoryPanel() {
  const state = useEditorState()
  const endRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    endRef.current?.scrollIntoView({ block: 'nearest' })
  }, [state.history.length])

  return (
    <Section
      title="History"
      right={<span className="num text-[9px] text-muted-foreground">{state.history.length} txns</span>}
    >
      <div className="pb-1">
        {state.history.map((h) => (
          <ContextMenu key={h.id}>
            <ContextMenuTrigger asChild>
              <div
                className={cn(
                  'flex h-5 items-center gap-1.5 px-2 text-[11px] text-muted-foreground',
                  state.undoTop === h.id && 'bg-primary/10 text-foreground',
                )}
                onDoubleClick={() => {
                  if (h.undoOf == null) core.cmd({ cmd: 'history-jump', id: h.id })
                }}
              >
                {h.undoOf != null && <Undo2 size={10} className="text-primary" />}
                <span className="truncate">{h.label}</span>
                <span className="num ml-auto text-[9px] opacity-50">#{h.id}</span>
              </div>
            </ContextMenuTrigger>
            <ContextMenuContent className="text-xs">
              <ContextMenuItem
                disabled={h.undoOf != null || state.undoTop === h.id}
                onClick={() => core.cmd({ cmd: 'history-jump', id: h.id })}
              >
                Revert to here
              </ContextMenuItem>
              <ContextMenuItem disabled={!state.canUndo} onClick={() => core.cmd({ cmd: 'undo' })}>
                Undo
              </ContextMenuItem>
              <ContextMenuItem disabled={!state.canRedo} onClick={() => core.cmd({ cmd: 'redo' })}>
                Redo
              </ContextMenuItem>
            </ContextMenuContent>
          </ContextMenu>
        ))}
        <div ref={endRef} />
      </div>
    </Section>
  )
}
