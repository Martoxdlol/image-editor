// Editor shell — the §14 layout: menu bar / document tabs / tool options /
// toolbar + canvas + panels / status bar. React renders chrome only; the
// document lives in the core worker (spec §12.1 golden rule).

import { useCallback, useState } from 'react'
import { TooltipProvider } from '@/components/ui/tooltip'
import CanvasView from './CanvasView'
import DocTabs from './DocTabs'
import MenuBar from './MenuBar'
import StatusBar from './StatusBar'
import Toolbar from './Toolbar'
import ToolOptions from './ToolOptions'
import ColorPanel from './panels/ColorPanel'
import HistoryPanel from './panels/HistoryPanel'
import LayersPanel from './panels/LayersPanel'
import PropertiesPanel from './panels/PropertiesPanel'
import { NewDocDialog, ExportDialog, ImportDialog, type PendingImport } from './dialogs'
import { useShortcuts } from './useShortcuts'

export default function Editor() {
  const [newDocOpen, setNewDocOpen] = useState(false)
  const [exportOpen, setExportOpen] = useState(false)
  const [pendingImport, setPendingImport] = useState<PendingImport | null>(null)

  const newDoc = useCallback(() => setNewDocOpen(true), [])
  const exportDlg = useCallback(() => setExportOpen(true), [])
  const importDlg = useCallback((p: PendingImport) => setPendingImport(p), [])
  useShortcuts({ newDoc, exportDlg })

  return (
    <TooltipProvider>
      <div className="flex h-screen w-screen flex-col overflow-hidden bg-background text-foreground">
        <MenuBar onNewDoc={newDoc} onExport={exportDlg} onImport={importDlg} />
        <DocTabs onNewDoc={newDoc} />
        <ToolOptions />
        <div className="flex min-h-0 flex-1">
          <Toolbar />
          <CanvasView />
          <div className="flex w-64 shrink-0 flex-col border-l bg-sidebar">
            <LayersPanel />
            <PropertiesPanel />
            <HistoryPanel />
            <ColorPanel />
          </div>
        </div>
        <StatusBar />
      </div>
      <NewDocDialog open={newDocOpen} onClose={() => setNewDocOpen(false)} />
      <ExportDialog open={exportOpen} onClose={() => setExportOpen(false)} />
      <ImportDialog pending={pendingImport} onClose={() => setPendingImport(null)} />
    </TooltipProvider>
  )
}
