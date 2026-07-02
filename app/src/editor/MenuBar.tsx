// Menu bar (spec §14): File Edit View Object Select Filter Window Help.

import {
  Menubar,
  MenubarContent,
  MenubarItem,
  MenubarMenu,
  MenubarSeparator,
  MenubarShortcut,
  MenubarSub,
  MenubarSubContent,
  MenubarSubTrigger,
  MenubarTrigger,
} from '@/components/ui/menubar'
import { core, downloadBytes } from '@/core/bridge'
import { copySelection, pasteFromClipboard } from './clipboard'
import { useEditorState } from './useEditorState'
import { DPR } from './CanvasView'

const FILTERS: { kind: string; label: string }[] = [
  { kind: 'filter.gaussian-blur', label: 'Gaussian Blur' },
  { kind: 'filter.pixelate', label: 'Pixelate' },
  { kind: 'filter.noise', label: 'Add Noise' },
]

const ADJUSTMENTS: { kind: string; label: string }[] = [
  { kind: 'adjust.brightness-contrast', label: 'Brightness / Contrast' },
  { kind: 'adjust.hsl', label: 'Hue / Saturation / Lightness' },
  { kind: 'adjust.levels', label: 'Levels' },
  { kind: 'adjust.invert', label: 'Invert' },
  { kind: 'adjust.grayscale', label: 'Grayscale' },
  { kind: 'adjust.posterize', label: 'Posterize' },
  { kind: 'adjust.threshold', label: 'Threshold' },
]

export default function MenuBar({
  onNewDoc,
  onExport,
}: {
  onNewDoc: () => void
  onExport: () => void
}) {
  const state = useEditorState()
  const sel = state.selection[0]

  const openFile = (accept: string, cb: (buf: ArrayBuffer, name: string) => void) => {
    const input = document.createElement('input')
    input.type = 'file'
    input.accept = accept
    input.onchange = async () => {
      const f = input.files?.[0]
      if (f) cb(await f.arrayBuffer(), f.name)
    }
    input.click()
  }

  const saveMyed = async () => {
    const bytes = await core.saveMyed()
    if (bytes.byteLength > 0) {
      downloadBytes(bytes, `${state.tabs[state.active]?.name ?? 'project'}.myed`, 'application/zip')
    }
  }

  const addModifier = (kind: string) => {
    if (sel) core.cmd({ cmd: 'add-modifier', node: sel, kind })
  }

  return (
    <Menubar className="h-8 rounded-none border-0 border-b bg-sidebar px-1">
      <div className="num mr-1 select-none px-2 text-[13px] font-semibold tracking-tight text-primary">
        ed<span className="text-muted-foreground">.</span>
      </div>
      <MenubarMenu>
        <MenubarTrigger className="text-xs">File</MenubarTrigger>
        <MenubarContent>
          <MenubarItem onClick={onNewDoc}>
            New… <MenubarShortcut>⌘N</MenubarShortcut>
          </MenubarItem>
          <MenubarItem onClick={() => openFile('.myed', (b, n) => core.openMyed(b, n))}>
            Open .myed… <MenubarShortcut>⌘O</MenubarShortcut>
          </MenubarItem>
          <MenubarSeparator />
          <MenubarItem onClick={saveMyed}>
            Save .myed <MenubarShortcut>⌘S</MenubarShortcut>
          </MenubarItem>
          <MenubarSeparator />
          <MenubarItem
            onClick={() =>
              openFile('image/*', (b, n) => core.importImage(b, n))
            }
          >
            Import Image…
          </MenubarItem>
          <MenubarItem onClick={onExport}>
            Export… <MenubarShortcut>⇧⌘E</MenubarShortcut>
          </MenubarItem>
        </MenubarContent>
      </MenubarMenu>
      <MenubarMenu>
        <MenubarTrigger className="text-xs">Edit</MenubarTrigger>
        <MenubarContent>
          <MenubarItem disabled={!state.canUndo} onClick={() => core.cmd({ cmd: 'undo' })}>
            Undo <MenubarShortcut>⌘Z</MenubarShortcut>
          </MenubarItem>
          <MenubarItem disabled={!state.canRedo} onClick={() => core.cmd({ cmd: 'redo' })}>
            Redo <MenubarShortcut>⇧⌘Z</MenubarShortcut>
          </MenubarItem>
          <MenubarSeparator />
          <MenubarItem disabled={!sel} onClick={() => copySelection(false)}>
            Copy <MenubarShortcut>⌘C</MenubarShortcut>
          </MenubarItem>
          <MenubarItem disabled={!sel} onClick={() => copySelection(true)}>
            Cut <MenubarShortcut>⌘X</MenubarShortcut>
          </MenubarItem>
          <MenubarItem onClick={() => pasteFromClipboard(false)}>
            Paste <MenubarShortcut>⌘V</MenubarShortcut>
          </MenubarItem>
          <MenubarItem disabled={!state.clipboardFull} onClick={() => pasteFromClipboard(true)}>
            Paste in Place <MenubarShortcut>⇧⌘V</MenubarShortcut>
          </MenubarItem>
          <MenubarItem disabled={!sel} onClick={() => core.cmd({ cmd: 'duplicate-selection' })}>
            Duplicate <MenubarShortcut>⌘D</MenubarShortcut>
          </MenubarItem>
          <MenubarSeparator />
          <MenubarItem disabled={!sel} onClick={() => core.cmd({ cmd: 'delete-selection' })}>
            Delete <MenubarShortcut>⌫</MenubarShortcut>
          </MenubarItem>
        </MenubarContent>
      </MenubarMenu>
      <MenubarMenu>
        <MenubarTrigger className="text-xs">View</MenubarTrigger>
        <MenubarContent>
          <MenubarItem onClick={() => core.cmd({ cmd: 'zoom-by', factor: 1.25, cx: 400, cy: 300 })}>
            Zoom In <MenubarShortcut>⌘+</MenubarShortcut>
          </MenubarItem>
          <MenubarItem onClick={() => core.cmd({ cmd: 'zoom-by', factor: 0.8, cx: 400, cy: 300 })}>
            Zoom Out <MenubarShortcut>⌘−</MenubarShortcut>
          </MenubarItem>
          <MenubarItem onClick={() => core.cmd({ cmd: 'set-view', zoom: DPR })}>
            Zoom 100% <MenubarShortcut>⌘1</MenubarShortcut>
          </MenubarItem>
          <MenubarItem onClick={() => core.cmd({ cmd: 'fit-view' })}>
            Fit <MenubarShortcut>⇧1</MenubarShortcut>
          </MenubarItem>
          <MenubarSeparator />
          <MenubarItem
            onClick={() => core.cmd({ cmd: 'set-pixel-preview', on: !state.view.pixelPreview })}
          >
            {state.view.pixelPreview ? '✓ ' : ''}Pixel Preview
          </MenubarItem>
        </MenubarContent>
      </MenubarMenu>
      <MenubarMenu>
        <MenubarTrigger className="text-xs">Object</MenubarTrigger>
        <MenubarContent>
          <MenubarItem disabled={!sel} onClick={() => core.cmd({ cmd: 'group-selection' })}>
            Group <MenubarShortcut>⌘G</MenubarShortcut>
          </MenubarItem>
          <MenubarItem disabled={!sel} onClick={() => core.cmd({ cmd: 'ungroup-selection' })}>
            Ungroup <MenubarShortcut>⇧⌘G</MenubarShortcut>
          </MenubarItem>
          <MenubarSeparator />
          <MenubarItem disabled={!sel} onClick={() => core.cmd({ cmd: 'rasterize-selection' })}>
            Rasterize (Subtree → Bitmap)
          </MenubarItem>
          <MenubarItem disabled={!sel} onClick={() => core.cmd({ cmd: 'convert-to-path' })}>
            Convert Shape → Path
          </MenubarItem>
          <MenubarSeparator />
          <MenubarSub>
            <MenubarSubTrigger disabled={!sel}>Add Modifier</MenubarSubTrigger>
            <MenubarSubContent>
              <MenubarItem onClick={() => addModifier('transform')}>Transform</MenubarItem>
              <MenubarItem onClick={() => addModifier('clip')}>Clip</MenubarItem>
            </MenubarSubContent>
          </MenubarSub>
          <MenubarSeparator />
          <MenubarItem onClick={() => core.cmd({ cmd: 'new-artboard', width: 800, height: 600 })}>
            New Artboard
          </MenubarItem>
        </MenubarContent>
      </MenubarMenu>
      <MenubarMenu>
        <MenubarTrigger className="text-xs">Select</MenubarTrigger>
        <MenubarContent>
          <MenubarItem onClick={() => core.cmd({ cmd: 'select-all' })}>
            Select All <MenubarShortcut>⌘A</MenubarShortcut>
          </MenubarItem>
          <MenubarItem
            disabled={!state.hasPixelSelection}
            onClick={() => core.cmd({ cmd: 'clear-pixel-selection' })}
          >
            Deselect <MenubarShortcut>⌘⇧A</MenubarShortcut>
          </MenubarItem>
          <MenubarItem
            disabled={!state.hasPixelSelection}
            onClick={() => core.cmd({ cmd: 'invert-pixel-selection' })}
          >
            Invert Selection
          </MenubarItem>
        </MenubarContent>
      </MenubarMenu>
      <MenubarMenu>
        <MenubarTrigger className="text-xs">Filter</MenubarTrigger>
        <MenubarContent>
          {FILTERS.map((f) => (
            <MenubarItem key={f.kind} disabled={!sel} onClick={() => addModifier(f.kind)}>
              {f.label}
            </MenubarItem>
          ))}
          <MenubarSeparator />
          {ADJUSTMENTS.map((f) => (
            <MenubarItem key={f.kind} disabled={!sel} onClick={() => addModifier(f.kind)}>
              {f.label}
            </MenubarItem>
          ))}
        </MenubarContent>
      </MenubarMenu>
      <MenubarMenu>
        <MenubarTrigger className="text-xs">Window</MenubarTrigger>
        <MenubarContent>
          {state.tabs.map((t, i) => (
            <MenubarItem key={i} onClick={() => core.cmd({ cmd: 'switch-doc', index: i })}>
              {i === state.active ? '✓ ' : ''}
              {t.name}
              {t.dirty ? ' •' : ''}
            </MenubarItem>
          ))}
        </MenubarContent>
      </MenubarMenu>
      <MenubarMenu>
        <MenubarTrigger className="text-xs">Help</MenubarTrigger>
        <MenubarContent>
          <MenubarItem
            onClick={() =>
              window.alert(
                'ed — a non-destructive, tree-based image editor.\nRust core (wasm) + tiny-skia CPU renderer.\n\nEverything you see is evaluated from the document tree; nothing is baked.',
              )
            }
          >
            About ed
          </MenubarItem>
        </MenubarContent>
      </MenubarMenu>
    </Menubar>
  )
}
