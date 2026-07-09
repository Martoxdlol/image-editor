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

import { MODIFIER_GROUPS } from './modifiers'

const FILTERS = MODIFIER_GROUPS.find((g) => g.group === 'Filters')!.items
const ADJUSTMENTS = MODIFIER_GROUPS.find((g) => g.group === 'Adjustments')!.items

export default function MenuBar({
  onNewDoc,
  onExport,
  onImport,
}: {
  onNewDoc: () => void
  onExport: () => void
  onImport: (p: import('./dialogs').PendingImport) => void
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
              openFile('image/*', (bytes, name) => {
                const url = URL.createObjectURL(new Blob([bytes]))
                const img = new Image()
                img.onload = () =>
                  onImport({ bytes, name, url, width: img.naturalWidth, height: img.naturalHeight })
                img.onerror = () => {
                  URL.revokeObjectURL(url)
                  core.importImage(bytes, name)
                }
                img.src = url
              })
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
          <MenubarItem
            disabled={!sel && !state.hasPixelSelection}
            onClick={() => copySelection(false)}
          >
            Copy {state.hasPixelSelection ? 'Area' : ''} <MenubarShortcut>⌘C</MenubarShortcut>
          </MenubarItem>
          <MenubarItem
            disabled={!sel && !state.hasPixelSelection}
            onClick={() => copySelection(true)}
          >
            Cut {state.hasPixelSelection ? 'Area' : ''} <MenubarShortcut>⌘X</MenubarShortcut>
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
          <MenubarItem
            disabled={!state.hasPixelSelection}
            onClick={() => core.cmd({ cmd: 'crop-to-selection' })}
          >
            Crop Image to Selection
          </MenubarItem>
          <MenubarItem
            disabled={!state.props.some((p) => p.kind === 'bitmap' && p.params['crop-w'] !== undefined)}
            onClick={() => core.cmd({ cmd: 'reset-crop' })}
          >
            Reset Image Crop
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
          <MenubarItem onClick={() => core.cmd({ cmd: 'new-artboard', width: 4000, height: 3000 })}>
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
                'A non-destructive, tree-based image editor.\nRust core (wasm) + tiny-skia CPU renderer.\n\nEverything you see is evaluated from the document tree; nothing is baked.',
              )
            }
          >
            About
          </MenubarItem>
        </MenubarContent>
      </MenubarMenu>
    </Menubar>
  )
}
