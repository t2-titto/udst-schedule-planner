import type { PCourse, PlannerState } from './model'

/**
 * The schedule the student built in this browser (choices, removed, unlocked, pins, courses added from the catalog, and combinations hidden
 * by a refusal), kept for the term it was built for, so a page refresh does not rebuild it from the suggestion. PeopleSoft is still read
 * again before Apply, so a stale copy cannot be applied silently.
 */
export const PICKS_KEY = 'schedule-planner:v2:picks'

export interface SavedPicks {
  term: string
  state: PlannerState
  added: PCourse[]
  excluded: Record<string, string[]>
}

const isStringArray = (x: unknown): x is string[] => Array.isArray(x) && x.every((v) => typeof v === 'string')

/** The saved copy, or null when there is none or it is not in the shape this version writes. */
export function readPicks(storage: Pick<Storage, 'getItem'> = window.localStorage): SavedPicks | null {
  try {
    const raw = storage.getItem(PICKS_KEY)
    if (!raw) return null
    const v = JSON.parse(raw) as Partial<SavedPicks> | null
    if (!v || typeof v.term !== 'string' || !v.state || typeof v.state !== 'object') return null
    const s = v.state as Partial<PlannerState>
    if (!isStringArray(s.removed) || !isStringArray(s.unlocked) || !Array.isArray(s.pins) || !s.choice || typeof s.choice !== 'object') return null
    if (!Array.isArray(v.added)) return null
    if (!v.excluded || typeof v.excluded !== 'object') return null
    return { term: v.term, state: s as PlannerState, added: v.added as PCourse[], excluded: v.excluded as Record<string, string[]> }
  } catch {
    return null
  }
}

/** Keep the schedule for the term. Storage can be blocked: the planner then simply starts from the suggestion, as before. */
export function writePicks(p: SavedPicks, storage: Pick<Storage, 'setItem'> = window.localStorage): void {
  try {
    storage.setItem(PICKS_KEY, JSON.stringify(p))
  } catch {
    /* storage blocked */
  }
}
