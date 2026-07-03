// Slider + directly-editable number, side by side. The slider previews
// while dragging; both the slider release and the field commit through
// `onCommit` (one history txn for document params).

import { Input } from '@/components/ui/input'
import { Slider } from '@/components/ui/slider'
import { useEffect, useState } from 'react'

export default function NumSlider({
  value,
  min,
  max,
  step,
  percent = false,
  className,
  onPreview,
  onCommit,
}: {
  value: number
  min: number
  max: number
  step?: number
  /** Display/edit as 0–100% while the underlying value is 0–1. */
  percent?: boolean
  className?: string
  onPreview: (v: number) => void
  onCommit: (v: number) => void
}) {
  const display = percent ? Math.round(value * 100) : Math.round(value * 100) / 100
  const [text, setText] = useState(String(display))
  useEffect(() => setText(String(display)), [display])

  const commitText = () => {
    let n = parseFloat(text)
    if (Number.isNaN(n)) {
      setText(String(display))
      return
    }
    if (percent) n /= 100
    n = Math.min(max, Math.max(min, n))
    onCommit(n)
  }

  return (
    <div className={`flex items-center gap-1.5 ${className ?? ''}`}>
      <Slider
        className="flex-1"
        min={min}
        max={max}
        step={step ?? (max - min) / 100}
        value={[value]}
        onValueChange={([v]) => onPreview(v)}
        onValueCommit={([v]) => onCommit(v)}
      />
      <div className="relative">
        <Input
          className="num h-5 w-12 px-1 pr-3.5 text-right text-[10px]"
          value={text}
          onChange={(e) => setText(e.target.value)}
          onBlur={commitText}
          onKeyDown={(e) => {
            if (e.key === 'Enter') (e.target as HTMLInputElement).blur()
            if (e.key === 'Escape') setText(String(display))
          }}
        />
        {percent && (
          <span className="pointer-events-none absolute right-1 top-1/2 -translate-y-1/2 text-[9px] text-muted-foreground">
            %
          </span>
        )}
      </div>
    </div>
  )
}
