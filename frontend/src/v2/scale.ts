import type { Settings } from '../engine/types'

/** The engine scores 0..10 with 5 neutral; the page shows value - 5. */
export const NEUTRAL_ENGINE = 5
export const toUi = (engine: number) => engine - NEUTRAL_ENGINE
export const fromUi = (ui: number) => ui + NEUTRAL_ENGINE

/** A page-scale score as the text shown to the person: "+2", "-1.5", "0". */
export function formatScore(ui: number): string {
  if (ui === 0) return '0'
  const text = Number.isInteger(ui) ? String(Math.abs(ui)) : Math.abs(ui).toFixed(1)
  return `${ui > 0 ? '+' : '-'}${text}`
}

/** Minutes after midnight as "2:00 PM". */
export function formatTime(minute: number): string {
  const m = Math.round(minute)
  const h24 = Math.floor(m / 60) % 24
  const mm = m % 60
  const h12 = h24 % 12 === 0 ? 12 : h24 % 12
  return `${h12}:${String(mm).padStart(2, '0')} ${h24 < 12 ? 'AM' : 'PM'}`
}

export function freeDaysMatter(weightEngine: number): boolean {
  return toUi(weightEngine) > -5
}

/** Settings as the solver should see them: when free days do not matter at all, no minimum number of free days is enforced. */
export function effectiveSettings(s: Settings): Settings {
  return freeDaysMatter(s.weights.freeDay) ? s : { ...s, rules: { ...s.rules, minFreeDays: 0 } }
}
