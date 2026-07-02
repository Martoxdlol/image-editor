// Left tool rail (spec §14). Grouped: select/move · shapes · pen/text ·
// raster · selections · utility.

import { core } from '@/core/bridge'
import { cn } from '@/lib/utils'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import type { ToolKind } from '@/core/types'
import { useEditorState } from './useEditorState'
import {
  MousePointer2,
  Square,
  Circle,
  Hexagon,
  Star,
  Minus,
  MoveUpRight,
  PenTool,
  Type,
  Brush,
  Pencil,
  Eraser,
  PaintBucket,
  Pipette,
  Blend,
  BoxSelect,
  LassoSelect,
  Wand2,
  Hand,
  ZoomIn,
  type LucideIcon,
} from 'lucide-react'

interface ToolDef {
  tool: ToolKind
  icon: LucideIcon
  label: string
  key?: string
}

const GROUPS: ToolDef[][] = [
  [{ tool: 'select', icon: MousePointer2, label: 'Move / Select', key: 'V' }],
  [
    { tool: 'rect', icon: Square, label: 'Rectangle', key: 'R' },
    { tool: 'ellipse', icon: Circle, label: 'Ellipse', key: 'O' },
    { tool: 'polygon', icon: Hexagon, label: 'Polygon' },
    { tool: 'star', icon: Star, label: 'Star' },
    { tool: 'line', icon: Minus, label: 'Line', key: 'L' },
    { tool: 'arrow', icon: MoveUpRight, label: 'Arrow' },
  ],
  [
    { tool: 'pen', icon: PenTool, label: 'Pen', key: 'P' },
    { tool: 'text', icon: Type, label: 'Text', key: 'T' },
  ],
  [
    { tool: 'brush', icon: Brush, label: 'Brush', key: 'B' },
    { tool: 'pencil', icon: Pencil, label: 'Pencil' },
    { tool: 'eraser', icon: Eraser, label: 'Eraser', key: 'E' },
    { tool: 'fill', icon: PaintBucket, label: 'Fill', key: 'G' },
    { tool: 'gradient', icon: Blend, label: 'Gradient' },
    { tool: 'eyedropper', icon: Pipette, label: 'Eyedropper', key: 'I' },
  ],
  [
    { tool: 'sel-rect', icon: BoxSelect, label: 'Rectangle Select', key: 'M' },
    { tool: 'sel-ellipse', icon: Circle, label: 'Ellipse Select' },
    { tool: 'lasso', icon: LassoSelect, label: 'Lasso' },
    { tool: 'wand', icon: Wand2, label: 'Magic Wand', key: 'W' },
  ],
  [
    { tool: 'pan', icon: Hand, label: 'Pan', key: 'H' },
    { tool: 'zoom', icon: ZoomIn, label: 'Zoom', key: 'Z' },
  ],
]

export default function Toolbar() {
  const state = useEditorState()
  return (
    <div className="flex w-10 shrink-0 flex-col items-center gap-0.5 border-r bg-sidebar py-1.5">
      {GROUPS.map((group, gi) => (
        <div key={gi} className="flex flex-col items-center gap-0.5">
          {gi > 0 && <div className="my-1 h-px w-5 bg-border" />}
          {group.map(({ tool, icon: Icon, label, key }) => (
            <Tooltip key={tool} delayDuration={400}>
              <TooltipTrigger asChild>
                <button
                  onClick={() => core.cmd({ cmd: 'set-tool', tool })}
                  className={cn(
                    'flex h-7 w-7 items-center justify-center rounded-sm text-muted-foreground transition-colors',
                    'hover:bg-accent hover:text-accent-foreground',
                    state.tool === tool && 'bg-primary/20 text-primary',
                  )}
                >
                  <Icon size={15} strokeWidth={1.75} />
                </button>
              </TooltipTrigger>
              <TooltipContent side="right" className="text-xs">
                {label}
                {key ? <span className="num ml-2 text-muted-foreground">{key}</span> : null}
              </TooltipContent>
            </Tooltip>
          ))}
        </div>
      ))}
    </div>
  )
}

export const TOOL_SHORTCUTS: Record<string, ToolKind> = {
  v: 'select',
  r: 'rect',
  o: 'ellipse',
  l: 'line',
  p: 'pen',
  t: 'text',
  b: 'brush',
  e: 'eraser',
  g: 'fill',
  i: 'eyedropper',
  m: 'sel-rect',
  w: 'wand',
  h: 'pan',
  z: 'zoom',
}
