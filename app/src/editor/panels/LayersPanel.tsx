// Tree/Layers panel (spec §14): artboards top-level, per-node visibility/
// lock, modifier badges, context menu (group, rasterize, convert, delete).

import { core } from '@/core/bridge'
import { cn } from '@/lib/utils'
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from '@/components/ui/context-menu'
import type { OutlineNode } from '@/core/types'
import {
  ChevronDown,
  ChevronRight,
  Eye,
  EyeOff,
  Frame,
  Image,
  Layers,
  Lock,
  LockOpen,
  PenLine,
  Pipette,
  Shapes,
  Sparkles,
  Type as TypeIcon,
  Waypoints,
} from 'lucide-react'
import { useState } from 'react'
import { useEditorState } from '../useEditorState'

const KIND_ICON: Record<string, typeof Frame> = {
  artboard: Frame,
  group: Layers,
  layer: Layers,
  shape: Shapes,
  path: Waypoints,
  text: TypeIcon,
  bitmap: Image,
  'stroke-set': PenLine,
  'gradient-fill': Pipette,
  reference: Sparkles,
}

function Row({ node, depth }: { node: OutlineNode; depth: number }) {
  const state = useEditorState()
  const [open, setOpen] = useState(true)
  const selected = state.selection.includes(node.id)
  const Icon = KIND_ICON[node.kind] ?? Shapes
  const hasKids = node.children.length > 0

  return (
    <>
      <ContextMenu>
        <ContextMenuTrigger asChild>
          <div
            onClick={(e) =>
              core.cmd({ cmd: 'select-nodes', ids: [node.id], toggle: e.shiftKey })
            }
            className={cn(
              'group flex h-6 cursor-default select-none items-center gap-1 pr-1 text-xs',
              selected ? 'bg-primary/20 text-foreground' : 'hover:bg-accent/50',
              !node.visible && 'opacity-45',
            )}
            style={{ paddingLeft: depth * 12 + 4 }}
          >
            <button
              className={cn('flex h-4 w-4 items-center justify-center text-muted-foreground', !hasKids && 'invisible')}
              onClick={(e) => {
                e.stopPropagation()
                setOpen(!open)
              }}
            >
              {open ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
            </button>
            <Icon size={12} className={cn('shrink-0', node.kind === 'artboard' ? 'text-primary' : 'text-muted-foreground')} />
            <span className="flex-1 truncate">{node.name}</span>
            {node.modifierBadges.length > 0 && (
              <span
                className="num rounded-sm bg-primary/15 px-1 text-[9px] text-primary"
                title={node.modifierBadges.join(', ')}
              >
                fx{node.modifierBadges.length}
              </span>
            )}
            <button
              className={cn(
                'rounded p-0.5 text-muted-foreground hover:text-foreground',
                node.locked ? '' : 'invisible group-hover:visible',
              )}
              onClick={(e) => {
                e.stopPropagation()
                core.cmd({ cmd: 'set-param', node: node.id, path: 'locked', value: !node.locked })
              }}
            >
              {node.locked ? <Lock size={11} /> : <LockOpen size={11} />}
            </button>
            <button
              className={cn(
                'rounded p-0.5 text-muted-foreground hover:text-foreground',
                node.visible ? 'invisible group-hover:visible' : '',
              )}
              onClick={(e) => {
                e.stopPropagation()
                core.cmd({ cmd: 'set-param', node: node.id, path: 'visible', value: !node.visible })
              }}
            >
              {node.visible ? <Eye size={11} /> : <EyeOff size={11} />}
            </button>
          </div>
        </ContextMenuTrigger>
        <ContextMenuContent className="text-xs">
          <ContextMenuItem onClick={() => core.cmd({ cmd: 'group-selection' })}>Group</ContextMenuItem>
          <ContextMenuItem onClick={() => core.cmd({ cmd: 'ungroup-selection' })}>Ungroup</ContextMenuItem>
          <ContextMenuItem onClick={() => core.cmd({ cmd: 'duplicate-selection' })}>Duplicate</ContextMenuItem>
          <ContextMenuSeparator />
          <ContextMenuItem onClick={() => core.cmd({ cmd: 'rasterize-selection' })}>
            Rasterize
          </ContextMenuItem>
          <ContextMenuItem onClick={() => core.cmd({ cmd: 'convert-to-path' })}>
            Convert to Path
          </ContextMenuItem>
          <ContextMenuSeparator />
          <ContextMenuItem
            onClick={() => {
              const name = window.prompt('Rename node', node.name)
              if (name) core.cmd({ cmd: 'set-param', node: node.id, path: 'name', value: name })
            }}
          >
            Rename…
          </ContextMenuItem>
          <ContextMenuItem
            variant="destructive"
            onClick={() => core.cmd({ cmd: 'delete-selection' })}
          >
            Delete
          </ContextMenuItem>
        </ContextMenuContent>
      </ContextMenu>
      {open &&
        // topmost first in the panel (reverse z-order)
        [...node.children].reverse().map((c) => <Row key={c.id} node={c} depth={depth + 1} />)}
    </>
  )
}

export default function LayersPanel() {
  const state = useEditorState()
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="panel-title px-2 py-1.5">Layers</div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {[...state.outline].reverse().map((n) => (
          <Row key={n.id} node={n} depth={0} />
        ))}
      </div>
    </div>
  )
}
