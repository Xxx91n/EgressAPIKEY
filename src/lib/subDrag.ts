/// Drag-sort helper for the SubscriptionsView list (ported from
/// D:/Aworker/env-manager/frontend/src/lib/profileDrag.ts — a minimal
/// mouse-event-based reorder with zero external deps, Ponytail full).
export interface DragState {
  dragIndex: number | null
  dragOverIndex: number | null
  isDragging: boolean
}

export function createDragState(): DragState {
  return { dragIndex: null, dragOverIndex: null, isDragging: false }
}

export function beginDrag(state: DragState, index: number, event?: { button?: number }): void {
  if (event?.button !== undefined && event.button !== 0) return
  state.dragIndex = index
  state.dragOverIndex = index
  state.isDragging = true
}

export function enterTarget(state: DragState, index: number): void {
  if (state.isDragging) state.dragOverIndex = index
}

export function finishDrag<T>(state: DragState, list: T[]): T[] {
  if (!state.isDragging || state.dragIndex === null || state.dragOverIndex === null) return list
  if (state.dragIndex === state.dragOverIndex) {
    cancelDrag(state)
    return list
  }
  const next = [...list]
  const [moved] = next.splice(state.dragIndex, 1)
  next.splice(state.dragOverIndex, 0, moved)
  cancelDrag(state)
  return next
}

export function cancelDrag(state: DragState): void {
  state.dragIndex = null
  state.dragOverIndex = null
  state.isDragging = false
}
