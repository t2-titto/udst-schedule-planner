import { useState } from 'react'
import {
  activeFreeDaysOf,
  countActivePrefs,
  DEFAULT_QUICK_SETUP,
  type QuickSetup,
  type SetupPacing,
  type SetupTimePref,
  type SuggestionCard,
} from './suggestions'

const clamp = (n: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, n))

const TILE_CONTRAST_ORDER = [0, 5, 4, 3, 7, 1, 2, 6] as const

const normTileKey = (code: string) => code.replace(/\s+/g, '').toUpperCase()

/**
 * Assign each course in the student's active/available list a distinct theme-matched tile color
 * first (no hash collisions across up to 8 courses), keeping consistent colors everywhere.
 */
export function assignTileClasses(primaryCodes: Iterable<string>, extraCodes?: Iterable<string>): Map<string, string> {
  const map = new Map<string, string>()
  const unique: string[] = []
  const push = (raw: string) => {
    const k = normTileKey(raw)
    if (k && !unique.includes(k)) unique.push(k)
  }
  const primary = [...primaryCodes].sort((a, b) => a.localeCompare(b))
  primary.forEach(push)
  if (extraCodes) {
    const extra = [...extraCodes].sort((a, b) => a.localeCompare(b))
    extra.forEach(push)
  }
  unique.forEach((k, idx) => {
    const slot = TILE_CONTRAST_ORDER[idx % TILE_CONTRAST_ORDER.length]
    map.set(k, `tile-${slot}`)
  })
  return map
}

export function tileClassFor(code: string, tileMap?: ReadonlyMap<string, string>): string {
  const k = normTileKey(code)
  const hit = tileMap?.get(k) ?? tileMap?.get(code)
  if (hit) return hit
  let h = 2166136261
  for (let i = 0; i < k.length; i++) h = Math.imul(h ^ k.charCodeAt(i), 16777619)
  return `tile-${TILE_CONTRAST_ORDER[(h >>> 0) % TILE_CONTRAST_ORDER.length]}`
}

const PREF_DAYS: { day: number; label: string }[] = [
  { day: 0, label: 'Sun' },
  { day: 1, label: 'Mon' },
  { day: 2, label: 'Tue' },
  { day: 3, label: 'Wed' },
  { day: 4, label: 'Thu' },
]

const PREF_TIMES: { val: SetupTimePref; label: string }[] = [
  { val: 'any', label: 'Any' },
  { val: 'morning', label: 'Morning' },
  { val: 'afternoon', label: 'Afternoon' },
  { val: 'evening', label: 'Evening' },
]

const PREF_PACING: { val: SetupPacing; label: string }[] = [
  { val: 'any', label: 'Balanced' },
  { val: 'compact', label: 'Compact' },
  { val: 'gapped', label: '30m breaks' },
]

interface Props {
  top: SuggestionCard[]
  more: SuggestionCard[]
  activeSig: string
  expanded: boolean
  onToggleExpand: () => void
  onPick: (card: SuggestionCard) => void
  unsolvableReason: string | null
  partialNotice?: string | null
  prefNotice?: string | null
  setup?: QuickSetup
  onChangeSetup?: (next: QuickSetup) => void
  tileMap?: ReadonlyMap<string, string>
}

function MiniGrid({ card, tileMap }: { card: SuggestionCard; tileMap?: ReadonlyMap<string, string> }) {
  const mins = card.schedule.blocks.flatMap((b) => [b.start, b.end])
  const startMin = Math.min(480, Math.floor((mins.length ? Math.min(...mins) : 480) / 60) * 60)
  const endMin = Math.max(1080, Math.ceil((mins.length ? Math.max(...mins) : 1080) / 60) * 60)
  const span = Math.max(60, endMin - startMin)
  const yCls = (m: number) => `py-${clamp(Math.round(((m - startMin) / span) * 200), 0, 198)}`
  const hCls = (s: number, e: number) => {
    const t = clamp(Math.round(((s - startMin) / span) * 200), 0, 198)
    const end = clamp(Math.round(((e - startMin) / span) * 200), t + 2, 200)
    return `ph-${clamp(end - t, 2, 200)}`
  }
  return (
    <div className="miniweek" aria-hidden="true">
      {[0, 1, 2, 3, 4].map((day) => {
        const blocks = card.schedule.blocks.filter((b) => b.day === day)
        return (
          <div key={day} className="miniweek-col">
            {blocks.map((b, i) => (
              <div key={i} className={`miniweek-block ${tileClassFor(b.courseCode, tileMap)} ${yCls(b.start)} ${hCls(b.start, b.end)}`} />
            ))}
          </div>
        )
      })}
    </div>
  )
}

export function SuggestionsStrip({
  top,
  more,
  activeSig,
  expanded,
  onToggleExpand,
  onPick,
  unsolvableReason,
  partialNotice,
  prefNotice,
  setup,
  onChangeSetup,
  tileMap,
}: Props) {
  const [prefsOpen, setPrefsOpen] = useState(false)
  if (top.length === 0 && !unsolvableReason && !setup) return null

  const prefCount = setup ? countActivePrefs(setup) : 0
  const selectedDays = setup ? activeFreeDaysOf(setup) : []

  const toggleFreeDay = (day: number) => {
    if (!setup || !onChangeSetup) return
    const nextDays = selectedDays.includes(day)
      ? selectedDays.filter((d) => d !== day)
      : [...selectedDays, day].sort((a, b) => a - b)
    onChangeSetup({
      ...setup,
      freeDays: nextDays,
      freeDay: nextDays.length === 1 ? (nextDays[0] as QuickSetup['freeDay']) : 'any',
    })
  }

  return (
    <section className="suggestions-bar" aria-label="Schedule suggestions">
      <div className="suggestions-head">
        <span className="sidebar-section-head">Suggestions</span>
        <div className="pill-row">
          {setup && onChangeSetup ? (
            <button
              type="button"
              className={`btn btn-sm ${prefsOpen || prefCount > 0 ? 'prefs-active-btn' : ''}`}
              aria-expanded={prefsOpen}
              onClick={() => setPrefsOpen((x) => !x)}
            >
              Preferences{prefCount > 0 ? ` (${prefCount})` : ''}
            </button>
          ) : null}
          {more.length > 0 ? (
            <button type="button" className="btn btn-sm" onClick={onToggleExpand}>
              {expanded ? 'Hide extra suggestions' : `Show more suggestions (${more.length})`}
            </button>
          ) : null}
        </div>
      </div>

      {prefsOpen && setup && onChangeSetup ? (
        <div className="prefs-panel" aria-label="Suggestion preferences">
          <div className="prefs-row">
            <span className="prefs-label">Free days</span>
            <div className="pill-row">
              {PREF_DAYS.map((d) => (
                <button
                  key={d.day}
                  type="button"
                  className={`choice-pill ${selectedDays.includes(d.day) ? 'active' : ''}`}
                  onClick={() => toggleFreeDay(d.day)}
                >
                  {d.label}
                </button>
              ))}
            </div>
          </div>

          <div className="prefs-row">
            <span className="prefs-label">Time window</span>
            <div className="pill-row">
              {PREF_TIMES.map((t) => (
                <button
                  key={t.val}
                  type="button"
                  className={`choice-pill ${setup.timePref === t.val ? 'active' : ''}`}
                  onClick={() => onChangeSetup({ ...setup, timePref: t.val })}
                >
                  {t.label}
                </button>
              ))}
              <button
                type="button"
                className={`choice-pill ${setup.noEarly8am ? 'active' : ''}`}
                onClick={() => onChangeSetup({ ...setup, noEarly8am: !setup.noEarly8am })}
              >
                No 8 AMs
              </button>
              <button
                type="button"
                className={`choice-pill ${setup.endBefore5pm ? 'active' : ''}`}
                onClick={() => onChangeSetup({ ...setup, endBefore5pm: !setup.endBefore5pm })}
              >
                End by 5 PM
              </button>
            </div>
          </div>

          <div className="prefs-row">
            <span className="prefs-label">Spacing &amp; rule</span>
            <div className="pill-row">
              {PREF_PACING.map((p) => (
                <button
                  key={p.val}
                  type="button"
                  className={`choice-pill ${(setup.pacing ?? 'any') === p.val ? 'active' : ''}`}
                  onClick={() => onChangeSetup({ ...setup, pacing: p.val })}
                >
                  {p.label}
                </button>
              ))}
              <label className="filter-chip" title="Try strictly matching selected days and times first, falling back with a notice if impossible.">
                <input
                  type="checkbox"
                  checked={!!setup.strict}
                  onChange={() => onChangeSetup({ ...setup, strict: !setup.strict })}
                />
                <span>Must match</span>
              </label>
              {prefCount > 0 ? (
                <button type="button" className="btn btn-sm" onClick={() => onChangeSetup({ ...DEFAULT_QUICK_SETUP })}>
                  Reset
                </button>
              ) : null}
            </div>
          </div>
        </div>
      ) : null}

      {prefNotice && !unsolvableReason && !partialNotice ? (
        <p className="calm-notice" role="status">{prefNotice}</p>
      ) : null}

      {unsolvableReason ? (
        <p className="calm-notice">{unsolvableReason}</p>
      ) : (
        <div className="suggestions-scroll">
          {top.map((card) => (
            <button
              key={card.id}
              type="button"
              className={`suggest-card ${activeSig === card.id ? 'active' : ''}`}
              onClick={() => onPick(card)}
            >
              <MiniGrid card={card} tileMap={tileMap} />
              <span className="suggest-title">{card.label}</span>
              {card.detail ? (
                <span className={`suggest-sub num ${card.skippedCodes?.length ? 'warn' : card.isOriginal ? 'ok' : ''}`}>
                  {card.detail}
                </span>
              ) : null}
            </button>
          ))}
          {more.map((card) => (
            <button
              key={card.id}
              type="button"
              disabled={!expanded}
              title={expanded ? `Apply ${card.label}` : 'Click Show more suggestions to unlock'}
              className={`suggest-card ${activeSig === card.id ? 'active' : ''} ${!expanded ? 'locked-more' : ''}`}
              onClick={() => expanded && onPick(card)}
            >
              <MiniGrid card={card} tileMap={tileMap} />
              <span className="suggest-title">{card.label}</span>
              {card.detail ? (
                <span className={`suggest-sub num ${card.skippedCodes?.length ? 'warn' : card.isOriginal ? 'ok' : ''}`}>
                  {card.detail}
                </span>
              ) : null}
            </button>
          ))}
        </div>
      )}
    </section>
  )
}
