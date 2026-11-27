/** The saved copies of the student's academic data in this browser. Settings (colours, weights) are not data of the account and stay. */
export const ACADEMIC_KEYS = [
  'schedule-planner:v2:degree',
  'schedule-planner:v2:home',
  'schedule-planner:v2:cache',
  'schedule-planner:v2:catalog',
  'schedule-planner:v2:prereqs',
  'schedule-planner:v2:picks',
  'schedule-planner:v1',
  'planie:workspace:v1',
] as const

export const WORKSPACE_CACHE_PREFIX = 'planie:workspace:v2:'
export const PREF_CACHE_PREFIX = 'planie:prefs:v1:'

/** On sign-out or account switch: remove saved copies of courses, terms, units, and enrolment state. */
export function clearAcademicData(storage: Pick<Storage, 'removeItem'> & Partial<Pick<Storage, 'length' | 'key'>> = window.localStorage): void {
  for (const k of ACADEMIC_KEYS) {
    try { storage.removeItem(k) } catch { /* storage blocked */ }
  }
  try {
    if (typeof storage.length === 'number' && typeof storage.key === 'function') {
      const toRemove: string[] = []
      for (let i = 0; i < storage.length; i++) {
        const k = storage.key(i)
        if (k && k.startsWith(WORKSPACE_CACHE_PREFIX)) toRemove.push(k)
      }
      for (const k of toRemove) storage.removeItem(k)
    }
  } catch { /* storage blocked */ }
}
