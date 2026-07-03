// Tree/Layers panel (spec §14): artboards top-level, per-node visibility/
// lock, modifier badges, context menu, and drag-to-reorder/reparent.
// The panel shows topmost-first; the model orders children bottom-to-top,
// so display row i maps to model index (len − 1 − i).

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
import Section from './Section'

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

const CONTAINER_KINDS = new Set(['artboard', 'group', 'layer'])

type DropPos = 'above' | 'below' | 'inside'

interface DndState {
  dragId: string | null
  dragIsArtboard: boolean
  drop: { id: string; pos: DropPos } | null
}

interface Dnd extends DndState {
  set: (patch: Partial<DndState>) => void
}

/** Is `id` inside the subtree rooted at `node`? (cycle guard) */
function subtreeContains(node: OutlineNode, id: string): boolean {
  if (node.id === id) return true
  return node.children.some((c) => subtreeContains(c, id))
}

function findNode(roots: OutlineNode[], id: string): OutlineNode | null {
  for (const r of roots) {
    if (r.id === id) return r
    const hit = findNode(r.children, id)
    if (hit) return hit
  }
  return null
}

/** Locate a node's parent id and model index (children are model-ordered). */
function findParentIndex(
  roots: OutlineNode[],
  id: string,
  parent: string | null = null,
): { parent: string | null; index: number } | null {
  for (let i = 0; i < roots.length; i++) {
    if (roots[i].id === id) return { parent, index: i }
    const hit = findParentIndex(roots[i].children, id, roots[i].id)
    if (hit) return hit
  }
  return null
}

function Row({
  node,
  depth,
  parentId,
  modelIndex,
  dnd,
  roots,
}: {
  node: OutlineNode
  depth: number
  /** null = top level (pasteboard). */
  parentId: string | null
  /** Index of this node in its parent's MODEL order (bottom-to-top). */
  modelIndex: number
  dnd: Dnd
  roots: OutlineNode[]
}) {
  const state = useEditorState()
  const [open, setOpen] = useState(true)
  const selected = state.selection.includes(node.id)
  const Icon = KIND_ICON[node.kind] ?? Shapes
  const hasKids = node.children.length > 0
  const isArtboard = node.kind === 'artboard'
  const isContainer = CONTAINER_KINDS.has(node.kind)
  const dropHere = dnd.drop?.id === node.id ? dnd.drop.pos : null

  const validDrop = (pos: DropPos): boolean => {
    if (!dnd.dragId || dnd.dragId === node.id) return false
    // no dropping a node into its own subtree
    const dragged = findNode(roots, dnd.dragId)
    if (dragged && subtreeContains(dragged, node.id)) return false
    if (dnd.dragIsArtboard) {
      // artboards live at the top level only
      if (pos === 'inside' || parentId !== null) return false
    } else if (pos === 'inside' && !isContainer) {
      return false
    }
    // no-op drops (back into the same slot) get no indicator
    const src = findParentIndex(roots, dnd.dragId)
    if (src) {
      if (pos === 'inside') {
        if (src.parent === node.id && src.index === node.children.length - 1) return false
      } else {
        const insert = pos === 'above' ? modelIndex + 1 : modelIndex
        if (src.parent === parentId && (insert === src.index || insert === src.index + 1)) {
          return false
        }
      }
    }
    return true
  }

  const onDragOver = (e: React.DragEvent) => {
    if (!dnd.dragId) return
    e.preventDefault()
    e.stopPropagation()
    const rect = e.currentTarget.getBoundingClientRect()
    const rel = (e.clientY - rect.top) / rect.height
    let pos: DropPos
    if (isContainer && !dnd.dragIsArtboard) {
      pos = rel < 0.28 ? 'above' : rel > 0.72 ? 'below' : 'inside'
    } else {
      pos = rel < 0.5 ? 'above' : 'below'
    }
    if (!validDrop(pos)) {
      // containers still accept above/below even when inside is invalid
      if (pos === 'inside' && validDrop('above')) pos = rel < 0.5 ? 'above' : 'below'
      if (!validDrop(pos)) {
        if (dnd.drop) dnd.set({ drop: null })
        return
      }
    }
    if (dnd.drop?.id !== node.id || dnd.drop.pos !== pos) {
      dnd.set({ drop: { id: node.id, pos } })
    }
  }

  const onDrop = (e: React.DragEvent) => {
    e.preventDefault()
    e.stopPropagation()
    const dragId = dnd.dragId
    const drop = dnd.drop
    dnd.set({ dragId: null, drop: null, dragIsArtboard: false })
    if (!dragId || !drop || drop.id !== node.id) return
    if (drop.pos === 'inside') {
      // append at model end = topmost inside the container
      core.cmd({ cmd: 'move-node', node: dragId, parent: node.id, index: node.children.length })
    } else {
      // above in the panel = higher z = later in model order
      const index = drop.pos === 'above' ? modelIndex + 1 : modelIndex
      core.cmd({ cmd: 'move-node', node: dragId, parent: parentId, index })
    }
  }

  return (
    <>
      <ContextMenu>
        <ContextMenuTrigger asChild>
          <div
            draggable
            onDragStart={(e) => {
              e.stopPropagation()
              e.dataTransfer.effectAllowed = 'move'
              e.dataTransfer.setData('text/plain', node.id)
              dnd.set({ dragId: node.id, dragIsArtboard: isArtboard, drop: null })
            }}
            onDragEnd={() => dnd.set({ dragId: null, drop: null, dragIsArtboard: false })}
            onDragOver={onDragOver}
            onDrop={onDrop}
            onClick={(e) =>
              core.cmd({ cmd: 'select-nodes', ids: [node.id], toggle: e.shiftKey })
            }
            className={cn(
              'group relative flex h-6 cursor-default select-none items-center gap-1 pr-1 text-xs',
              selected ? 'bg-primary/20 text-foreground' : 'hover:bg-accent/50',
              !node.visible && 'opacity-45',
              dnd.dragId === node.id && 'opacity-40',
              dropHere === 'inside' && 'ring-1 ring-inset ring-primary bg-primary/10',
            )}
            style={{ paddingLeft: depth * 12 + 4 }}
          >
            {dropHere === 'above' && (
              <div className="pointer-events-none absolute inset-x-0 top-0 h-0.5 bg-primary" />
            )}
            {dropHere === 'below' && (
              <div className="pointer-events-none absolute inset-x-0 bottom-0 h-0.5 bg-primary" />
            )}
            <button
              className={cn(
                'flex h-4 w-4 items-center justify-center text-muted-foreground',
                !hasKids && 'invisible',
              )}
              onClick={(e) => {
                e.stopPropagation()
                setOpen(!open)
              }}
            >
              {open ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
            </button>
            <Icon
              size={12}
              className={cn('shrink-0', isArtboard ? 'text-primary' : 'text-muted-foreground')}
            />
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
        [...node.children]
          .reverse()
          .map((c, di) => (
            <Row
              key={c.id}
              node={c}
              depth={depth + 1}
              parentId={node.id}
              modelIndex={node.children.length - 1 - di}
              dnd={dnd}
              roots={roots}
            />
          ))}
    </>
  )
}

export default function LayersPanel() {
  const state = useEditorState()
  const [dndState, setDndState] = useState<DndState>({
    dragId: null,
    dragIsArtboard: false,
    drop: null,
  })
  const dnd: Dnd = {
    ...dndState,
    set: (patch) => setDndState((s) => ({ ...s, ...patch })),
  }
  return (
    <Section title="Layers">
      {[...state.outline].reverse().map((n, di) => (
        <Row
          key={n.id}
          node={n}
          depth={0}
          parentId={null}
          modelIndex={state.outline.length - 1 - di}
          dnd={dnd}
          roots={state.outline}
        />
      ))}
    </Section>
  )
}
