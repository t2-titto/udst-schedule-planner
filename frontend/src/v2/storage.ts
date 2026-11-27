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
  'planie:favTeachers',
] as const

export const WORKSPACE_CACHE_PREFIX = 'planie:workspace:v2:'
export const PREF_CACHE_PREFIX = 'planie:prefs:v1:'
export const FAV_TEACHERS_PREFIX = 'planie:favTeachers:'
export const SCHEDULE_CACHE_PREFIX = 'schedule-planner:v2:cache:'

type StorageLike = Pick<Storage, 'removeItem'> & Partial<Pick<Storage, 'length' | 'key'>>

const ACADEMIC_PREFIXES = [
  WORKSPACE_CACHE_PREFIX,
  PREF_CACHE_PREFIX,
  FAV_TEACHERS_PREFIX,
  SCHEDULE_CACHE_PREFIX,
] as const

function clearStorageArea(area: StorageLike): void {
  for (const k of ACADEMIC_KEYS) {
    try { area.removeItem(k) } catch { /* storage blocked */ }
  }
  try {
    if (typeof area.length === 'number' && typeof area.key === 'function') {
      const toRemove: string[] = []
      for (let i = 0; i < area.length; i++) {
        const k = area.key(i)
        if (k && ACADEMIC_PREFIXES.some((p) => k.startsWith(p))) toRemove.push(k)
      }
      for (const k of toRemove) area.removeItem(k)
    }
  } catch { /* storage blocked */ }
}

/** On sign-out or account switch: remove saved copies of courses, terms, units, preferences, and enrolment state. */
export function clearAcademicData(storage?: StorageLike): void {
  if (storage) {
    clearStorageArea(storage)
    return
  }
  try { clearStorageArea(window.localStorage) } catch { /* storage blocked */ }
  try { clearStorageArea(window.sessionStorage) } catch { /* storage blocked */ }
}
