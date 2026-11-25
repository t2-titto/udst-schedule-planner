import { useState } from 'react'
import { instructorKey, NEUTRAL, type Course } from '../engine/types'
import { CenteredSlider } from './CenteredSlider'
import { instructorStats, type ClassInfo } from './instructors'
import { toUi, fromUi } from './scale'

interface Props {
  courses: Course[]
  scores: Record<string, number>
  onScore: (key: string, engine: number) => void
  pins: string[]
  onPin: (courseCode: string, classNbr: string) => void
  idPrefix: string
}

const more = (n: number, shown: number) => (n > shown ? ` +${n - shown} more` : '')

function linkText(c: ClassInfo): string {
  const shown = c.links.slice(0, 3)
  const list = shown.map((l) => `${l.nbr} (${l.when}, ${l.instructors.join(' & ')})`).join('; ')
  if (c.links.length === 0) return 'not picked together with another class'
  return `${c.isLab ? 'goes with lecture' : 'goes with lab'}${c.links.length > 1 ? 's' : ''} ${list}${more(c.links.length, shown.length)}`
}

/**
 * Instructor scores. Each course opens when you hover (or focus, or click) its name; under every instructor is how many classes they
 * teach, split into lectures and labs. Hover that line to see the classes: tick one to pin it, so it is always part of the schedule,
 * with the lecture or lab it goes with and who teaches it.
 */
export function InstructorPanel({ courses, scores, onScore, pins, onPin, idPrefix }: Props) {
  const [hover, setHover] = useState<string | null>(null)
  const [sticky, setSticky] = useState<string[]>([])
  const [hoverInst, setHoverInst] = useState<string | null>(null)
  const [stickyInst, setStickyInst] = useState<string[]>([])
  const toggle = (list: string[], v: string) => (list.includes(v) ? list.filter((x) => x !== v) : [...list, v])

  const withInstructors = courses.map((c) => ({ course: c, stats: instructorStats(c.options) })).filter((c) => c.stats.length > 0)
  if (withInstructors.length === 0) return <p className="pref-empty">No instructors yet.</p>

  return (
    <div className="ins">
      {withInstructors.map(({ course, stats }) => {
        const open = hover === course.code || sticky.includes(course.code)
        const pinned = stats.flatMap((s) => s.classes).filter((c, i, a) => a.findIndex((x) => x.nbr === c.nbr) === i && pins.includes(c.nbr)).length
        return (
          <div key={course.code} className="ins-course" role="group" aria-label={course.code} onMouseEnter={() => setHover(course.code)} onMouseLeave={() => setHover((h) => (h === course.code ? null : h))}>
            <button type="button" className="ins-course__head" aria-expanded={open} onFocus={() => setHover(course.code)} onClick={() => setSticky((s) => toggle(s, course.code))}>
              <span className="ins-course__code">{course.code}</span>
              <span className="ins-course__title">{course.title}</span>
              <span className="ins-course__n">{stats.length} instructor{stats.length > 1 ? 's' : ''}{pinned ? ` · ${pinned} pinned` : ''}</span>
            </button>
            {open ? (
              <div className="ins-course__body">
                {stats.map((st, i) => {
                  const key = instructorKey(course.code, st.name)
                  const showList = hoverInst === key || stickyInst.includes(key)
                  const total = st.classes.length
                  return (
                    <div key={key} className="ins-inst">
                      <CenteredSlider id={`${idPrefix}-${course.code}-${i}`} label={st.name} value={toUi(scores[key] ?? NEUTRAL)} onChange={(v) => onScore(key, fromUi(v))} />
                      <button
                        type="button"
                        className="ins-counts"
                        aria-expanded={showList}
                        onMouseEnter={() => setHoverInst(key)}
                        onMouseLeave={() => setHoverInst((h) => (h === key ? null : h))}
                        onFocus={() => setHoverInst(key)}
                        onClick={() => setStickyInst((s) => toggle(s, key))}
                      >
                        {total} class{total === 1 ? '' : 'es'} · {st.lectures} lecture{st.lectures === 1 ? '' : 's'} · {st.labs} lab{st.labs === 1 ? '' : 's'}
                      </button>
                      {showList ? (
                        <ul className="ins-list" onMouseEnter={() => setHoverInst(key)} onMouseLeave={() => setHoverInst((h) => (h === key ? null : h))}>
                          {st.classes.map((c) => (
                            <li key={c.nbr} className={pins.includes(c.nbr) ? 'is-pinned' : undefined}>
                              <label>
                                <input type="checkbox" checked={pins.includes(c.nbr)} onChange={() => onPin(course.code, c.nbr)} aria-label={`Pin ${c.isLab ? 'lab' : 'lecture'} ${c.nbr}`} />
                                <span className="ins-list__main"><strong>{c.isLab ? 'Lab' : 'Lecture'} {c.nbr}</strong> · {c.when}</span>
                              </label>
                              <span className="ins-list__link">{linkText(c)}</span>
                            </li>
                          ))}
                          <li className="ins-list__hint">Tick to pin: it becomes the top priority and the schedule is rebuilt around it.</li>
                        </ul>
                      ) : null}
                    </div>
                  )
                })}
              </div>
            ) : null}
          </div>
        )
      })}
    </div>
  )
}
