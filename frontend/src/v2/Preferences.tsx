import { useEffect, useId, useState } from 'react'
import { DAY_SHORT } from '../engine/types'
import type { Course, Rules, Settings } from '../engine/types'
import { CenteredSlider } from './CenteredSlider'
import { InstructorPanel } from './InstructorPanel'
import type { Fix } from './fixes'
import type { Outlier } from './outlier'
import { formatTime, freeDaysMatter, fromUi, toUi } from './scale'
import './preferences.css'

export interface PreferencesPanelProps {
  pins: string[]
  onPin: (courseCode: string, classNbr: string) => void
  outlier: Outlier | null
  onFix: (fix: Fix) => void
  onClose: () => void
  settings: Settings
  patch: (fn: (s: Settings) => Settings) => void
  courses: Course[]
}

const WEEK = [0, 1, 2, 3, 4]
const GAPS = [0, 5, 10, 15, 20, 30, 45, 60]
const LATEST_HOURS = Array.from({ length: 15 }, (_, i) => 9 + i)
const NEUTRAL = 5
const OPEN_KEY = 'v2-pref-open'
const TIME_STEPS = Array.from({ length: 35 }, (_, i) => 6 * 60 + i * 30) // 6:00 AM .. 11:00 PM
const FADES = Array.from({ length: 13 }, (_, i) => i * 30) // 0 .. 6 h

function loadOpen(): string[] {
  try {
    const raw = window.localStorage.getItem(OPEN_KEY)
    const v = raw ? (JSON.parse(raw) as unknown) : null
    if (Array.isArray(v)) return v.filter((x): x is string => typeof x === 'string')
  } catch { /* storage blocked */ }
  return ['time']
}

function Section({ id, title, open, toggle, children }: { id: string; title: string; open: boolean; toggle: (id: string) => void; children: React.ReactNode }) {
  const uid = useId()
  return (
    <section className="pref-section">
      <h3 className="pref-heading pref-heading--disclosure">
        <button type="button" className="pref-disclosure" aria-expanded={open} aria-controls={`${uid}-body`} onClick={() => toggle(id)}>
          {title}
          <svg className="pref-chevron" width="16" height="16" viewBox="0 0 16 16" aria-hidden="true">
            <path d="M5 3l5 5-5 5" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        </button>
      </h3>
      <div id={`${uid}-body`} hidden={!open}>{children}</div>
    </section>
  )
}

/** Preferences as a docked panel beside the calendar. Each section opens and closes on its own and stays as left. */
export function PreferencesPanel({ onClose, settings, patch, courses, pins, onPin, outlier, onFix }: PreferencesPanelProps) {
  const titleId = useId()
  const [openIds, setOpenIds] = useState<string[]>(loadOpen)
  useEffect(() => {
    try { window.localStorage.setItem(OPEN_KEY, JSON.stringify(openIds)) } catch { /* ignore */ }
  }, [openIds])
  const toggle = (id: string) => setOpenIds((o) => (o.includes(id) ? o.filter((x) => x !== id) : [...o, id]))
  const isOpen = (id: string) => openIds.includes(id)

  const timing = settings.timing
  const setTiming = (change: Partial<typeof timing>) => patch((st) => ({ ...st, timing: { ...st.timing, ...change } }))
  const timeOptions = (v: number) => (TIME_STEPS.includes(v) ? TIME_STEPS : [...TIME_STEPS, v].sort((x, y) => x - y))
  const freeMatter = freeDaysMatter(settings.weights.freeDay)

  function setWeight(key: 'instructor' | 'timing' | 'freeDay', ui: number) {
    patch((s) => ({ ...s, weights: { ...s.weights, [key]: fromUi(ui) } }))
  }

  function setFreeDay(day: number, ui: number) {
    patch((s) => {
      const freeDayScores = [...s.freeDayScores]
      while (freeDayScores.length < 7) freeDayScores.push(NEUTRAL)
      freeDayScores[day] = fromUi(ui)
      return { ...s, freeDayScores }
    })
  }

  function setRule(change: Partial<Rules>) {
    patch((s) => ({ ...s, rules: { ...s.rules, ...change } }))
  }

  const rules = settings.rules
  const latestOptions = rules.latestEnd !== null && !LATEST_HOURS.includes(rules.latestEnd / 60) ? [rules.latestEnd] : []

  return (
    <aside className="pref-panel" aria-labelledby={titleId}>
      <header className="pref-head">
        <h2 id={titleId} className="pref-title">Preferences</h2>
        <button type="button" className="pref-close" onClick={onClose} aria-label="Close preferences">
          <svg width="18" height="18" viewBox="0 0 18 18" aria-hidden="true">
            <path d="M3 3l12 12M15 3L3 15" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
          </svg>
        </button>
      </header>
      <div className="pref-body">
        <Section id="time" title="Time of day" open={isOpen('time')} toggle={toggle}>
          <div className="pref-field">
            <label htmlFor={`${titleId}-from`}>Best from</label>
            <select id={`${titleId}-from`} value={timing.start} onChange={(e) => setTiming({ start: Number(e.currentTarget.value) })}>
              {timeOptions(timing.start).map((m) => <option key={m} value={m}>{formatTime(m)}</option>)}
            </select>
          </div>
          <div className="pref-field">
            <label htmlFor={`${titleId}-until`}>Until</label>
            <select id={`${titleId}-until`} value={timing.end} onChange={(e) => setTiming({ end: Number(e.currentTarget.value) })}>
              {timeOptions(timing.end).map((m) => <option key={m} value={m}>{formatTime(m)}</option>)}
            </select>
          </div>
          <div className="pref-field">
            <label htmlFor={`${titleId}-fade`}>Fade out over</label>
            <select id={`${titleId}-fade`} value={timing.fade} onChange={(e) => setTiming({ fade: Number(e.currentTarget.value) })}>
              {(FADES.includes(timing.fade) ? FADES : [...FADES, timing.fade].sort((x, y) => x - y)).map((m) => <option key={m} value={m}>{m === 0 ? 'No fade' : `${m / 60} h`}</option>)}
            </select>
          </div>
          <p className="pref-empty">Classes inside the window score best; the score falls off gradually outside it.</p>
        </Section>

        <Section id="importance" title="Importance" open={isOpen('importance')} toggle={toggle}>
          <CenteredSlider id={`${titleId}-w-instructor`} label="Instructors" value={toUi(settings.weights.instructor)} onChange={(v) => setWeight('instructor', v)} />
          <CenteredSlider id={`${titleId}-w-timing`} label="Preferred time" value={toUi(settings.weights.timing)} onChange={(v) => setWeight('timing', v)} />
          <CenteredSlider id={`${titleId}-w-free`} label="Free days" value={toUi(settings.weights.freeDay)} onChange={(v) => setWeight('freeDay', v)} />
          {!freeMatter ? <p className="pref-empty">Free days do not matter (slider at the far left).</p> : null}
        </Section>

        {freeMatter ? (
          <Section id="free" title="Free days" open={isOpen('free')} toggle={toggle}>
            {WEEK.map((d) => (
              <CenteredSlider key={d} id={`${titleId}-day-${d}`} label={DAY_SHORT[d]} value={toUi(settings.freeDayScores[d] ?? NEUTRAL)} onChange={(v) => setFreeDay(d, v)} />
            ))}
          </Section>
        ) : null}

        <Section id="instructors" title="Instructor scores" open={isOpen('instructors')} toggle={toggle}>
          <InstructorPanel courses={courses} scores={settings.instructorScores} onScore={(key, v) => patch((st) => ({ ...st, instructorScores: { ...st.instructorScores, [key]: v } }))} pins={pins} onPin={onPin} idPrefix={`${titleId}-i`} />
        </Section>

        <Section id="rules" title="Rules" open={isOpen('rules')} toggle={toggle}>
          <div className="pref-field">
            <label htmlFor={`${titleId}-gap`}>Minimum gap between classes</label>
            <select id={`${titleId}-gap`} value={rules.minGap} onChange={(e) => setRule({ minGap: Number(e.currentTarget.value) })}>
              {GAPS.map((g) => <option key={g} value={g}>{g === 0 ? 'None' : `${g} min`}</option>)}
            </select>
          </div>
          <div className="pref-field">
            <label htmlFor={`${titleId}-late`}>Latest end</label>
            <select
              id={`${titleId}-late`}
              value={rules.latestEnd === null ? '' : String(rules.latestEnd)}
              onChange={(e) => setRule({ latestEnd: e.currentTarget.value === '' ? null : Number(e.currentTarget.value) })}
            >
              <option value="">No limit</option>
              {[...LATEST_HOURS.map((h) => h * 60), ...latestOptions].sort((a, b) => a - b).map((m) => (
                <option key={m} value={m}>{formatTime(m)}</option>
              ))}
            </select>
          </div>
          {freeMatter ? (
            <div className="pref-field">
              <label htmlFor={`${titleId}-minfree`}>Minimum free days</label>
              <select id={`${titleId}-minfree`} value={rules.minFreeDays} onChange={(e) => setRule({ minFreeDays: Number(e.currentTarget.value) })}>
                {[0, 1, 2, 3].map((n) => <option key={n} value={n}>{n}</option>)}
              </select>
            </div>
          ) : null}
          <label className="pref-check">
            <input type="checkbox" checked={rules.excludeFull} onChange={(e) => setRule({ excludeFull: e.currentTarget.checked })} />
            Skip full classes
          </label>
          <label className="pref-check">
            <input type="checkbox" checked={rules.ignoreOnline} onChange={(e) => setRule({ ignoreOnline: e.currentTarget.checked })} />
            Don't score online classes
          </label>
          <label className="pref-check">
            <input type="checkbox" checked={!!rules.joinWaitlist} onChange={(e) => setRule({ joinWaitlist: e.currentTarget.checked })} />
            Waitlists (unverified)
          </label>
          <p className="pref-note">Treat waitlisted classes like enrolled ones, and offer to join a class's waitlist. Not tried on a real waitlist yet: PeopleSoft's own answer is always shown. Takes effect at the next refresh.</p>
        </Section>
      </div>
      {outlier ? (
        <footer className="pref-foot" aria-live="polite">
          <span>Small thing: {outlier.what} Without it the best schedule would score about {outlier.gain.toFixed(1)} higher.</span>
          <button type="button" className="pref-link" onClick={() => onFix(outlier.fix)}>{outlier.fix.label}</button>
        </footer>
      ) : null}
    </aside>
  )
}
