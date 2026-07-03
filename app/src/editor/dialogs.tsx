// New-document, import, and export dialogs (spec §9 import/export options,
// §2.1 artboard params).

import { useEffect, useMemo, useState } from 'react'
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'
import { Slider } from '@/components/ui/slider'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { core, downloadBytes } from '@/core/bridge'
import { cn } from '@/lib/utils'
import { ArrowLeftRight } from 'lucide-react'
import { useEditorState } from './useEditorState'

const Row = ({ label, children }: { label: string; children: React.ReactNode }) => (
  <label className="flex items-center gap-2">
    <span className="w-24 shrink-0 text-[11px] text-muted-foreground">{label}</span>
    {children}
  </label>
)

// ------------------------------------------------------------------ new

const PRESETS: { label: string; w: number; h: number; hint?: string }[] = [
  { label: '4000 × 3000', w: 4000, h: 3000, hint: 'default' },
  { label: '1920 × 1080', w: 1920, h: 1080, hint: 'full HD' },
  { label: '1080 × 1080', w: 1080, h: 1080, hint: 'social' },
  { label: '1080 × 1920', w: 1080, h: 1920, hint: 'story' },
  { label: '2480 × 3508', w: 2480, h: 3508, hint: 'A4 300dpi' },
  { label: '800 × 600', w: 800, h: 600 },
  { label: '512 × 512', w: 512, h: 512, hint: 'icon' },
  { label: '64 × 64', w: 64, h: 64, hint: 'pixel art' },
]

export function NewDocDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [name, setName] = useState('')
  const [w, setW] = useState('4000')
  const [h, setH] = useState('3000')
  const [dpi, setDpi] = useState('72')
  const [background, setBackground] = useState('color')
  const [bgColor, setBgColor] = useState('#ffffff')

  const create = () => {
    const width = parseFloat(w)
    const height = parseFloat(h)
    if (!(width > 0) || !(height > 0)) return
    core.cmd({
      cmd: 'new-doc',
      width,
      height,
      name: name.trim() || null,
      background,
      bg_color: bgColor,
      dpi: parseFloat(dpi) || 72,
    })
    onClose()
  }

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-sm">
        <DialogHeader>
          <DialogTitle className="text-sm">New document</DialogTitle>
        </DialogHeader>
        <div className="space-y-2.5">
          <Row label="Name">
            <Input
              className="h-7 text-xs"
              placeholder="Untitled"
              value={name}
              onChange={(e) => setName(e.target.value)}
            />
          </Row>
          <Row label="Size (px)">
            <Input className="num h-7" value={w} onChange={(e) => setW(e.target.value)} />
            <button
              className="shrink-0 rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
              title="Swap width/height"
              onClick={() => {
                setW(h)
                setH(w)
              }}
            >
              <ArrowLeftRight size={12} />
            </button>
            <Input className="num h-7" value={h} onChange={(e) => setH(e.target.value)} />
          </Row>
          <Row label="DPI">
            <Input className="num h-7 w-20" value={dpi} onChange={(e) => setDpi(e.target.value)} />
          </Row>
          <Row label="Background">
            <Select value={background} onValueChange={setBackground}>
              <SelectTrigger size="sm" className="h-7 flex-1 text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {['color', 'transparent', 'checker'].map((b) => (
                  <SelectItem key={b} value={b} className="text-xs">
                    {b}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {background === 'color' && (
              <input
                type="color"
                value={bgColor}
                onChange={(e) => setBgColor(e.target.value)}
                className="h-7 w-9 cursor-pointer rounded border bg-transparent"
              />
            )}
          </Row>
          <div className="grid grid-cols-2 gap-1.5 pt-1">
            {PRESETS.map((p) => (
              <button
                key={p.label}
                onClick={() => {
                  setW(String(p.w))
                  setH(String(p.h))
                }}
                className={cn(
                  'flex items-baseline justify-between rounded border px-2 py-1 text-left hover:bg-accent',
                  w === String(p.w) && h === String(p.h) && 'border-primary bg-primary/10',
                )}
              >
                <span className="num text-[11px]">{p.label}</span>
                {p.hint && <span className="text-[9px] text-muted-foreground">{p.hint}</span>}
              </button>
            ))}
          </div>
        </div>
        <DialogFooter>
          <Button size="sm" onClick={create}>
            Create
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

// ------------------------------------------------------------------ import

export interface PendingImport {
  bytes: ArrayBuffer
  name: string
  url: string
  width: number
  height: number
}

export function ImportDialog({
  pending,
  onClose,
}: {
  pending: PendingImport | null
  onClose: () => void
}) {
  const [placement, setPlacement] = useState<'original' | 'fit' | 'custom'>('original')
  const [pct, setPct] = useState('50')
  const [asNewDoc, setAsNewDoc] = useState(false)

  useEffect(() => {
    if (pending) {
      setPlacement('original')
      setAsNewDoc(false)
    }
  }, [pending])

  if (!pending) return null

  const doImport = () => {
    const scale =
      placement === 'fit' ? -1 : placement === 'custom' ? (parseFloat(pct) || 100) / 100 : 1
    core.importImage(pending.bytes, pending.name, { scale, newDoc: asNewDoc })
    URL.revokeObjectURL(pending.url)
    onClose()
  }

  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-sm">
        <DialogHeader>
          <DialogTitle className="text-sm">Import image</DialogTitle>
        </DialogHeader>
        <div className="space-y-2.5">
          <div className="flex items-center gap-3">
            <img
              src={pending.url}
              alt=""
              className="h-20 w-20 rounded border object-contain"
              style={{
                background:
                  'repeating-conic-gradient(#ccc 0% 25%, #fff 0% 50%) 0 0 / 12px 12px',
              }}
            />
            <div className="min-w-0 text-[11px] text-muted-foreground">
              <div className="truncate text-foreground">{pending.name}</div>
              <div className="num">
                {pending.width} × {pending.height} px
              </div>
              <div className="num">{(pending.bytes.byteLength / 1024).toFixed(0)} KB</div>
            </div>
          </div>
          <div className="space-y-1">
            {(
              [
                ['original', 'Original size'],
                ['fit', 'Fit artboard'],
                ['custom', 'Custom scale'],
              ] as const
            ).map(([key, label]) => (
              <label key={key} className="flex cursor-pointer items-center gap-2 text-xs">
                <input
                  type="radio"
                  name="placement"
                  checked={placement === key}
                  onChange={() => setPlacement(key)}
                  disabled={asNewDoc}
                />
                {label}
                {key === 'custom' && placement === 'custom' && (
                  <span className="flex items-center gap-1">
                    <Input
                      className="num h-6 w-14 px-1 text-[11px]"
                      value={pct}
                      onChange={(e) => setPct(e.target.value)}
                    />
                    <span className="text-muted-foreground">%</span>
                  </span>
                )}
              </label>
            ))}
          </div>
          <label className="flex cursor-pointer items-center gap-2 text-xs">
            <Checkbox checked={asNewDoc} onCheckedChange={(v) => setAsNewDoc(v === true)} />
            Open as new document ({pending.width} × {pending.height})
          </label>
        </div>
        <DialogFooter>
          <Button size="sm" onClick={doImport}>
            Import
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

// ------------------------------------------------------------------ export

export function ExportDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const state = useEditorState()
  const [artboard, setArtboard] = useState('0')
  const [scale, setScale] = useState('1')
  const [format, setFormat] = useState('png')
  const [quality, setQuality] = useState(90)
  const [background, setBackground] = useState(true)
  const [filename, setFilename] = useState('')
  const [busy, setBusy] = useState(false)

  const ab = state.artboards[parseInt(artboard)] ?? state.artboards[0]
  const scaleNum = parseFloat(scale) || 1
  const outW = ab ? Math.round(ab.w * scaleNum) : 0
  const outH = ab ? Math.round(ab.h * scaleNum) : 0
  const defaultName = useMemo(
    () => `${(ab?.name ?? 'artboard').replace(/\s+/g, '-')}@${scaleNum}x`,
    [ab?.name, scaleNum],
  )
  const mime = format === 'png' ? 'image/png' : format === 'jpeg' ? 'image/jpeg' : 'image/webp'

  const exportOne = async (index: number, nameBase: string) => {
    const bytes = await core.exportArtboard(index, scaleNum, format, {
      background,
      quality,
    })
    if (bytes.byteLength > 0) downloadBytes(bytes, `${nameBase}.${format}`, mime)
  }

  const doExport = async () => {
    setBusy(true)
    try {
      await exportOne(parseInt(artboard), filename.trim() || defaultName)
      onClose()
    } finally {
      setBusy(false)
    }
  }

  const doExportAll = async () => {
    setBusy(true)
    try {
      for (const a of state.artboards) {
        await exportOne(a.index, `${a.name.replace(/\s+/g, '-')}@${scaleNum}x`)
      }
      onClose()
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-sm">
        <DialogHeader>
          <DialogTitle className="text-sm">Export</DialogTitle>
        </DialogHeader>
        <div className="space-y-2.5">
          <Row label="Artboard">
            <Select value={artboard} onValueChange={setArtboard}>
              <SelectTrigger size="sm" className="h-7 flex-1 text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {state.artboards.map((a) => (
                  <SelectItem key={a.index} value={String(a.index)} className="text-xs">
                    {a.name}{' '}
                    <span className="num text-muted-foreground">
                      {Math.round(a.w)}×{Math.round(a.h)}
                    </span>
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Row>
          <Row label="Format">
            <Select value={format} onValueChange={setFormat}>
              <SelectTrigger size="sm" className="h-7 flex-1 text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {['png', 'jpeg', 'webp'].map((f) => (
                  <SelectItem key={f} value={f} className="text-xs">
                    {f.toUpperCase()}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Row>
          <Row label="Scale">
            <div className="flex flex-1 items-center gap-1">
              {['0.5', '1', '2', '4'].map((s) => (
                <button
                  key={s}
                  onClick={() => setScale(s)}
                  className={cn(
                    'num rounded border px-2 py-0.5 text-[11px] hover:bg-accent',
                    scale === s && 'border-primary bg-primary/10',
                  )}
                >
                  {s}×
                </button>
              ))}
              <Input
                className="num h-7 w-16 text-[11px]"
                value={scale}
                onChange={(e) => setScale(e.target.value)}
              />
            </div>
          </Row>
          {format === 'jpeg' && (
            <Row label="Quality">
              <Slider
                className="flex-1"
                min={1}
                max={100}
                step={1}
                value={[quality]}
                onValueChange={([v]) => setQuality(v)}
              />
              <span className="num w-8 text-right text-[11px]">{quality}</span>
            </Row>
          )}
          <Row label="Background">
            <label className="flex items-center gap-2 text-xs text-muted-foreground">
              <Checkbox checked={background} onCheckedChange={(v) => setBackground(v === true)} />
              include artboard background
            </label>
          </Row>
          <Row label="File name">
            <Input
              className="h-7 flex-1 text-xs"
              placeholder={defaultName}
              value={filename}
              onChange={(e) => setFilename(e.target.value)}
            />
            <span className="text-[11px] text-muted-foreground">.{format}</span>
          </Row>
          <div className="num rounded border bg-card/60 px-2 py-1 text-[11px] text-muted-foreground">
            output: {outW} × {outH} px
          </div>
        </div>
        <DialogFooter className="gap-2">
          <Button size="sm" variant="secondary" disabled={busy || state.artboards.length < 2} onClick={doExportAll}>
            Export all ({state.artboards.length})
          </Button>
          <Button size="sm" disabled={busy} onClick={doExport}>
            {busy ? 'Rendering…' : 'Export'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
