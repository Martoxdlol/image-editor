// VS Code-style collapsible panel section: chevron header, content fills
// available space when open, collapse state persisted per title.

import { cn } from '@/lib/utils'
import { ChevronRight } from 'lucide-react'
import { useState, type ReactNode } from 'react'

export default function Section({
  title,
  right,
  grow = true,
  children,
}: {
  title: string
  right?: ReactNode
  /** Expanded section takes a share of the column (flex-1). */
  grow?: boolean
  children: ReactNode
}) {
  const [open, setOpen] = useState(() => localStorage.getItem(`panel:${title}`) !== '0')
  const toggle = () => {
    setOpen((o) => {
      localStorage.setItem(`panel:${title}`, o ? '0' : '1')
      return !o
    })
  }
  return (
    <div
      className={cn(
        'flex flex-col border-t first:border-t-0',
        open && grow ? 'min-h-0 flex-1' : 'shrink-0',
      )}
    >
      <button
        onClick={toggle}
        className="flex h-6 w-full shrink-0 items-center gap-1 px-1.5 text-left hover:bg-accent/40"
        aria-expanded={open}
      >
        <ChevronRight
          size={11}
          className={cn('shrink-0 text-muted-foreground transition-transform', open && 'rotate-90')}
        />
        <span className="panel-title">{title}</span>
        <span className="ml-auto flex items-center" onClick={(e) => e.stopPropagation()}>
          {right}
        </span>
      </button>
      {open && <div className="min-h-0 flex-1 overflow-y-auto">{children}</div>}
    </div>
  )
}
