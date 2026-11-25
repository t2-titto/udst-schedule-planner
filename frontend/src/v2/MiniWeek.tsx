import type { Schedule } from '../engine/solver'
import { DAY_SHORT } from '../engine/types'

interface Props {
  schedule: Schedule
  days: number[]
  /** Shared by every thumbnail so heights are comparable: [first minute, last minute]. */
  range: [number, number]
  colorOf: (code: string) => number
}

const W = 100
const HEAD = 11
const H = 72
const NOON = 12 * 60

/**
 * A small week at a glance: one column per day, one block per class in its course colour, empty space where
 * there is a gap, and a tinted column when the whole day is free.
 */
export function MiniWeek({ schedule, days, range: [first, last], colorOf }: Props) {
  const colW = W / days.length
  const y = (m: number) => HEAD + ((m - first) / (last - first)) * H
  const col = (d: number) => days.indexOf(d) * colW
  return (
    <svg className="mini" viewBox={`0 0 ${W} ${HEAD + H}`} aria-hidden="true" focusable="false">
      {days.map((d, i) => {
        const free = schedule.freeDays.includes(d)
        return (
          <g key={d}>
            <rect className={`mini-col${free ? ' is-free' : ''}`} x={i * colW + 0.5} y={HEAD} width={colW - 1} height={H} rx={2} />
            <text className={`mini-day${free ? ' is-free' : ''}`} x={i * colW + colW / 2} y={8} textAnchor="middle">{DAY_SHORT[d]}</text>
          </g>
        )
      })}
      {NOON > first && NOON < last ? <line className="mini-noon" x1={0.5} x2={W - 0.5} y1={y(NOON)} y2={y(NOON)} /> : null}
      {schedule.blocks.map((b) => (
        <rect
          key={`${b.courseCode}-${b.classNbr}-${b.day}-${b.start}`}
          className={`mini-block c${colorOf(b.courseCode)}${b.online ? ' is-online' : ''}`}
          x={col(b.day) + 1.5}
          y={y(b.start) + 0.4}
          width={colW - 3}
          height={Math.max(2, y(b.end) - y(b.start) - 0.8)}
          rx={1.5}
        />
      ))}
    </svg>
  )
}
