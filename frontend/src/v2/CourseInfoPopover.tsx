import type { PrereqStatus } from './compatible'

export interface CourseInfoTarget {
  code: string
  title: string
  units: number | null
  unlocks: string[]
  prereqText: string
  prereqStatus: PrereqStatus
  coreqs: string[]
  plannedTerm: string | null
  inSchedule: boolean
  openCount?: number
  waitlistCount?: number
  closedCount?: number
  onAdd?: () => void
  onAddWithCoreq?: () => void
  onRemove?: () => void
  onMoveTerm?: () => void
}

interface Props {
  target: CourseInfoTarget | null
  onClose: () => void
}

export function CourseInfoPopover({ target, onClose }: Props) {
  if (!target) return null
  return (
    <div className="sheet-backdrop" role="dialog" aria-modal="true" aria-label={`${target.code} details`} onClick={onClose}>
      <div className="sheet-card" onClick={(e) => e.stopPropagation()}>
        <div className="sheet-head">
          <div>
            <h3>{target.code}</h3>
            <p className="course-title">{target.title}{target.units != null ? ` · ${target.units} cr` : ''}</p>
          </div>
          <button type="button" className="btn btn-sm" onClick={onClose}>Close</button>
        </div>

        <div className="stage-list">
          <div className="stage-item">
            <strong>Unlocks</strong>
            <span>{target.unlocks.length > 0 ? target.unlocks.join(', ') : 'No later degree courses depend on this'}</span>
          </div>
          {target.prereqText ? (
            <div className="stage-item">
              <strong>Requires</strong>
              <span>{target.prereqText}</span>
            </div>
          ) : null}
          {target.coreqs.length > 0 ? (
            <div className="stage-item">
              <strong>Companion</strong>
              <span>Take together with {target.coreqs.join(', ')}</span>
            </div>
          ) : null}
          {target.plannedTerm ? (
            <div className="stage-item">
              <strong>Planned in</strong>
              <span>{target.plannedTerm}</span>
            </div>
          ) : null}
          {typeof target.openCount === 'number' ? (
            <div className="stage-item">
              <strong>Availability</strong>
              <span>
                {target.openCount} open
                {target.waitlistCount ? ` · ${target.waitlistCount} waitlist` : ''}
                {target.closedCount ? ` · ${target.closedCount} closed` : ''}
              </span>
            </div>
          ) : null}
        </div>

        <div className="pill-row">
          {target.onAddWithCoreq && target.coreqs.length > 0 ? (
            <button type="button" className="btn btn-primary" onClick={() => { target.onAddWithCoreq?.(); onClose() }}>
              Add both ({target.code} + {target.coreqs[0]})
            </button>
          ) : target.onAdd ? (
            <button type="button" className="btn btn-primary" onClick={() => { target.onAdd?.(); onClose() }}>
              {target.inSchedule ? 'Already in schedule' : 'Plan & add to schedule'}
            </button>
          ) : null}
          {target.onMoveTerm ? (
            <button type="button" className="btn" onClick={() => { target.onMoveTerm?.(); onClose() }}>
              Move term
            </button>
          ) : null}
          {target.onRemove ? (
            <button type="button" className="btn" onClick={() => { target.onRemove?.(); onClose() }}>
              Remove from schedule
            </button>
          ) : null}
        </div>
      </div>
    </div>
  )
}
