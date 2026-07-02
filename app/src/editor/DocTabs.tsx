// Document tabs (spec §14): name + unsaved dot, close on hover, [+].

import { core } from '@/core/bridge'
import { cn } from '@/lib/utils'
import { Plus, X } from 'lucide-react'
import { useEditorState } from './useEditorState'

export default function DocTabs({ onNewDoc }: { onNewDoc: () => void }) {
  const state = useEditorState()
  return (
    <div className="flex h-8 items-end gap-0 border-b bg-background/60 px-1">
      {state.tabs.map((tab, i) => (
        <div
          key={i}
          role="tab"
          aria-selected={i === state.active}
          onClick={() => core.cmd({ cmd: 'switch-doc', index: i })}
          onAuxClick={(e) => {
            if (e.button === 1) core.cmd({ cmd: 'close-doc', index: i })
          }}
          className={cn(
            'group relative flex h-7 max-w-44 cursor-default select-none items-center gap-1.5 rounded-t-md border border-b-0 px-3 text-xs',
            i === state.active
              ? 'bg-sidebar text-foreground'
              : 'border-transparent text-muted-foreground hover:bg-accent/40',
          )}
        >
          <span className="truncate">{tab.name}</span>
          {tab.dirty && <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-primary" />}
          {state.tabs.length > 1 && (
            <button
              onClick={(e) => {
                e.stopPropagation()
                core.cmd({ cmd: 'close-doc', index: i })
              }}
              className="invisible -mr-1 rounded p-0.5 hover:bg-accent group-hover:visible"
              aria-label={`Close ${tab.name}`}
            >
              <X size={11} />
            </button>
          )}
        </div>
      ))}
      <button
        onClick={onNewDoc}
        className="ml-1 flex h-6 w-6 items-center justify-center rounded text-muted-foreground hover:bg-accent hover:text-foreground"
        aria-label="New document"
      >
        <Plus size={13} />
      </button>
    </div>
  )
}
