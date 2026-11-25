/** The saved copies of the student's academic data in this browser. Settings (colours, weights) are not data of the account and stay. */
export const ACADEMIC_KEYS = [
  'schedule-planner:v2:degree',
  'schedule-planner:v2:home',
  'schedule-planner:v2:cache',
  'schedule-planner:v2:catalog',
  'schedule-planner:v2:prereqs',
  'schedule-planner:v2:picks',
  'schedule-planner:v1',
] as const

/** On sign-out: the saved copies of courses, terms, units and enrolment status go too. */
export function clearAcademicData(storage: Pick<Storage, 'removeItem'> = window.localStorage): void {
  for (const k of ACADEMIC_KEYS) {
    try { storage.removeItem(k) } catch { /* storage blocked */ }
  }
}
