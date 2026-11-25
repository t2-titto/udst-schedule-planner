import type { Credits } from './credits'
import { creditText } from './credits'

interface Props {
  credits: Credits
  /** Take an enrolled course out of the schedule (Apply then drops it). */
  onRemoveEnrolled: (code: string) => void
  /** The button's text (on the Apply screen it goes back to the planner). */
  removeLabel?: string
}

/** The credit numbers, every course with its credits, and the way out when the plan is over the limit. */
export function CreditPanel({ credits: c, onRemoveEnrolled, removeLabel = 'Take it out of my schedule' }: Props) {
  return (
    <section className="v2-credits" aria-label="Credits">
      <p className="v2-credits__sum">
        <strong>Schedule {creditText(c.schedule)}</strong> · <strong>PeopleSoft plan {creditText(c.after)}{c.unknown.length ? '+' : ''}</strong> / limit {c.limit.max}
        {c.limit.confirmed ? null : <em> ({c.limit.note})</em>}
      </p>
      <p className="v2-muted v2-credits__how">
        PeopleSoft counts every course planned for the term, even one you unticked here, plus what you are enrolled in. It refuses when you press Enrol above the limit.
      </p>
      {c.deferred.length ? (
        <p className="v2-muted v2-credits__how">
          Not counted in this term, because it would be over the limit otherwise: {c.deferred.map((d) => `${d.code} (PeopleSoft lists it here, but it belongs to ${d.to} in your degree)`).join(', ')}. PeopleSoft's own Enrol click has the last word.
        </p>
      ) : null}
      <table className="v2-credits__table">
        <thead><tr><th>Course</th><th>Credits</th><th>In your schedule</th><th>PeopleSoft counts it</th></tr></thead>
        <tbody>
          {c.lines.map((l) => (
            <tr key={l.code}>
              <td><strong>{l.code}</strong> <span className="v2-muted">{l.kind === 'enrolled' ? 'enrolled' : 'planned'}</span></td>
              <td>{l.units === null ? <em>unknown</em> : creditText(l.units)}</td>
              <td>{l.inSchedule ? 'yes' : 'no'}</td>
              <td>{l.counts ? 'yes' : l.kind === 'enrolled' ? 'no, Apply drops it' : 'no'}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {c.unknown.length ? <p className="v2-alert" role="alert">PeopleSoft did not give the credits of {c.unknown.join(', ')}, so the total cannot be trusted. Refresh, or check the course in PeopleSoft.</p> : null}
      {c.over ? (
        <div className="v2-alert" role="alert">
          <strong>{creditText(c.after)} credits is {creditText(c.excess)} over the {c.limit.max} PeopleSoft allows.</strong> Apply is blocked. Remove at least {creditText(c.excess)} credit{c.excess === 1 ? '' : 's'}:
          <ul>
            {c.wayOut.map((w) => (
              <li key={w.code}>
                <strong>{w.code}</strong> ({w.units === null ? '?' : creditText(w.units)} credits){w.enough ? ' – enough on its own' : ''}:{' '}
                {w.action === 'unplan' ? (
                  <>planned only. Move it to another term in "Plan my degree". PeopleSoft does not let a required course be left without a term, so a planned course can only be moved, not removed.</>
                ) : (
                  <>
                    you are enrolled, so this <strong>drops you from the course</strong> and the seat may be lost.{' '}
                    <button type="button" className="btn btn-quiet btn-sm" onClick={() => onRemoveEnrolled(w.code)}>{removeLabel}</button>
                  </>
                )}
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </section>
  )
}
