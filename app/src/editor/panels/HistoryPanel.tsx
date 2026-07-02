// History panel (spec §14): txn-level entries from the append-only op log
// (undo entries appear as new rows — history is never rewritten, §3.3).

import { Undo2 } from 'lucide-react'
import { useEffect, useRef } from 'react'
import { useEditorState } from '../useEditorState'

export default function HistoryPanel() {
  const state = useEditorState()
  const endRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    endRef.current?.scrollIntoView({ block: 'nearest' })
  }, [state.history.length])

  return (
    <div className="flex min-h-0 flex-1 flex-col border-t">
      <div className="panel-title flex items-center justify-between px-2 py-1.5">
        <span>History</span>
        <span className="num text-[9px]">{state.history.length} txns</span>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto pb-1">
        {state.history.map((h) => (
          <div
            key={h.id}
            className="flex h-5 items-center gap-1.5 px-2 text-[11px] text-muted-foreground"
          >
            {h.undoOf != null && <Undo2 size={10} className="text-primary" />}
            <span className="truncate">{h.label}</span>
            <span className="num ml-auto text-[9px] opacity-50">#{h.id}</span>
          </div>
        ))}
        <div ref={endRef} />
      </div>
    </div>
  )
}
