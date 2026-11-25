/** A bounded undo/redo history. `commit` with the same tag within `windowMs` replaces the newest step, so a slider drag is one step. */
export interface History<T> {
  past: T[]
  present: T
  future: T[]
  tag?: string
  at: number
}

export const HISTORY_LIMIT = 50
export const COALESCE_MS = 700

export const startHistory = <T,>(value: T): History<T> => ({ past: [], present: value, future: [], at: 0 })

export function commit<T>(h: History<T>, next: T, now: number, tag?: string, limit = HISTORY_LIMIT, windowMs = COALESCE_MS): History<T> {
  if (Object.is(next, h.present)) return h
  if (tag && h.tag === tag && now - h.at < windowMs && h.past.length > 0) return { ...h, present: next, future: [], at: now }
  const past = [...h.past, h.present]
  while (past.length > limit) past.shift()
  return { past, present: next, future: [], tag, at: now }
}

export function undo<T>(h: History<T>): History<T> {
  if (h.past.length === 0) return h
  const past = h.past.slice(0, -1)
  return { past, present: h.past[h.past.length - 1], future: [h.present, ...h.future], at: 0 }
}

export function redo<T>(h: History<T>): History<T> {
  if (h.future.length === 0) return h
  const [next, ...future] = h.future
  return { past: [...h.past, h.present], present: next, future, at: 0 }
}

/** A fresh history around a new value (used when a new load replaces everything). */
export const reset = <T,>(value: T): History<T> => startHistory(value)
