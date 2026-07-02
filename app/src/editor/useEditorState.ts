import { useSyncExternalStore } from 'react'
import { core } from '@/core/bridge'
import type { EditorState } from '@/core/types'

export function useEditorState(): EditorState {
  return useSyncExternalStore(core.subscribe, core.getState)
}
