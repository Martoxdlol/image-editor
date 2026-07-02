// New-document and export dialogs (spec §9 export options, §2.1 artboards).

import { useState } from 'react'
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { core, downloadBytes } from '@/core/bridge'
import { useEditorState } from './useEditorState'

export function NewDocDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [w, setW] = useState('800')
  const [h, setH] = useState('600')
  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-xs">
        <DialogHeader>
          <DialogTitle className="text-sm">New document</DialogTitle>
        </DialogHeader>
        <div className="flex items-center gap-2">
          <Input className="num h-7" value={w} onChange={(e) => setW(e.target.value)} />
          <span className="text-muted-foreground">×</span>
          <Input className="num h-7" value={h} onChange={(e) => setH(e.target.value)} />
          <span className="text-xs text-muted-foreground">px</span>
        </div>
        <div className="flex flex-wrap gap-1.5">
          {[
            ['800×600', 800, 600],
            ['1024×1024', 1024, 1024],
            ['1920×1080', 1920, 1080],
            ['64×64', 64, 64],
          ].map(([label, pw, ph]) => (
            <Button
              key={label as string}
              variant="secondary"
              size="sm"
              className="h-6 text-xs"
              onClick={() => {
                setW(String(pw))
                setH(String(ph))
              }}
            >
              {label}
            </Button>
          ))}
        </div>
        <DialogFooter>
          <Button
            size="sm"
            onClick={() => {
              const width = parseFloat(w)
              const height = parseFloat(h)
              if (width > 0 && height > 0) {
                core.cmd({ cmd: 'new-doc', width, height })
                onClose()
              }
            }}
          >
            Create
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

export function ExportDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const state = useEditorState()
  const [artboard, setArtboard] = useState('0')
  const [scale, setScale] = useState('1')
  const [format, setFormat] = useState('png')
  const [busy, setBusy] = useState(false)

  const doExport = async () => {
    setBusy(true)
    try {
      const bytes = await core.exportArtboard(parseInt(artboard), parseFloat(scale), format)
      if (bytes.byteLength > 0) {
        const name = `${state.artboards[parseInt(artboard)]?.name ?? 'artboard'}@${scale}x.${format}`
        const mime = format === 'png' ? 'image/png' : format === 'jpeg' ? 'image/jpeg' : 'image/webp'
        downloadBytes(bytes, name, mime)
      }
      onClose()
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-xs">
        <DialogHeader>
          <DialogTitle className="text-sm">Export artboard</DialogTitle>
        </DialogHeader>
        <div className="space-y-2">
          <Select value={artboard} onValueChange={setArtboard}>
            <SelectTrigger size="sm" className="w-full text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {state.artboards.map((a) => (
                <SelectItem key={a.index} value={String(a.index)} className="text-xs">
                  {a.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <div className="flex gap-2">
            <Select value={scale} onValueChange={setScale}>
              <SelectTrigger size="sm" className="flex-1 text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {['0.5', '1', '2', '4'].map((s) => (
                  <SelectItem key={s} value={s} className="text-xs">
                    {s}×
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <Select value={format} onValueChange={setFormat}>
              <SelectTrigger size="sm" className="flex-1 text-xs">
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
          </div>
        </div>
        <DialogFooter>
          <Button size="sm" disabled={busy} onClick={doExport}>
            {busy ? 'Rendering…' : 'Export'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
