import { useEffect, useRef, useState } from 'react'
import { fmtCompact } from '../engine/normalize'
import { DAY_NAMES } from '../engine/types'
import { nearestGuide, type Guide } from './guides'
import type { Overlap, PBlock } from './model'

const HOUR = 44
const INSET = 10

interface Props {
  blocks: PBlock[]
  overlaps: Overlap[]
  colorOf: (code: string) => number
  active?: string | null
  onPick?: (code: string) => void
  /** The real places a block can be dragged to (computed when the drag starts). Omit to turn dragging off. */
  guidesFor?: (b: PBlock) => Guide[]
  /** A block was dropped on a real place. */
  onDropGuide?: (b: PBlock, g: Guide) => void
  /** The place a drag is currently over (null when it ends), so the header can show that schedule's score. */
  onPreview?: (g: Guide | null) => void
  /** Score of the schedule now (engine scale), to show how far each place changes it. */
  baseScore?: number | null
  /** Draw every course muted (the Done screen). */
  quiet?: boolean
}

const kindShort = (k: string) => (/lab/i.test(k) ? 'Lab' : /lec/i.test(k) ? 'Lecture' : k)
const sameBlock = (a: PBlock, b: PBlock) => a.courseCode === b.courseCode && a.classNbr === b.classNbr && a.day === b.day && a.start === b.start
const involved = (b: PBlock, o: Overlap) => sameBlock(o.a, b) || sameBlock(o.b, b)
const blockKey = (b: PBlock) => `${b.courseCode}-${b.classNbr}-${b.day}-${b.start}`

export interface Lane { lane: number; lanes: number; cluster: string }

/**
 * Where each block sits when cards overlap. Cards that touch in time on one day form a cluster; inside it the longest card
 * is behind (lane 0) and shorter ones are in front, so a small card stays clickable even when it overlaps a big one.
 */
export function lanesOf(blocks: PBlock[]): Map<string, Lane> {
  const out = new Map<string, Lane>()
  for (const day of new Set(blocks.map((b) => b.day))) {
    const list = blocks.filter((b) => b.day === day).sort((a, b) => a.start - b.start || b.end - a.end)
    let group: PBlock[] = []
    let end = -1
    const flush = () => {
      if (group.length === 0) return
      const cluster = `${day}:${Math.min(...group.map((g) => g.start))}`
      const ordered = [...group].sort((a, b) => b.end - b.start - (a.end - a.start) || a.start - b.start)
      ordered.forEach((b, lane) => out.set(blockKey(b), { lane, lanes: ordered.length, cluster }))
      group = []
    }
    for (const b of list) {
      if (group.length && b.start >= end) { flush(); end = -1 }
      group.push(b)
      end = Math.max(end, b.end)
    }
    flush()
  }
  return out
}

interface Drag {
  b: PBlock
  guides: Guide[]
  x: number
  y: number
  dx: number
  dy: number
  grabMin: number
  moved: boolean
  near: Guide | null
}

/** The week, never hidden: an overlap is hatched in red right where it happens. */
export function Calendar({ blocks, overlaps, colorOf, active, onPick, guidesFor, onDropGuide, onPreview, baseScore, quiet }: Props) {
  const bodyRef = useRef<HTMLDivElement>(null)
  const [drag, setDrag] = useState<Drag | null>(null)
  const dragRef = useRef<Drag | null>(null)
  const suppressClick = useRef(false)
  const [spread, setSpread] = useState<string | null>(null)
  const set = (d: Drag | null) => { dragRef.current = d; setDrag(d) }

  const lanes = lanesOf(blocks)
  const guideDays = drag ? drag.guides.flatMap((g) => [...g.here, ...g.others].map((m) => m.day)) : []
  const days = [0, 1, 2, 3, 4, ...[5, 6].filter((d) => blocks.some((b) => b.day === d) || guideDays.includes(d))]
  const spans = [...blocks, ...(drag ? drag.guides.flatMap((g) => [...g.here, ...g.others]) : [])]
  const first = Math.min(8 * 60, ...spans.map((b) => Math.floor(b.start / 60) * 60))
  const last = Math.max(21 * 60, ...spans.map((b) => Math.ceil(b.end / 60) * 60))
  const hours: number[] = []
  for (let m = first; m < last; m += 60) hours.push(m)
  const height = hours.length * HOUR
  const top = (m: number) => ((m - first) / 60) * HOUR

  useEffect(() => {
    if (!drag && spread === null) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      if (dragRef.current) { set(null); onPreview?.(null); suppressClick.current = true } else setSpread(null)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [drag, spread, onPreview])

  /** Day column index and the minute under a screen point. */
  const locate = (x: number, y: number) => {
    const body = bodyRef.current
    if (!body) return null
    const rect = body.getBoundingClientRect()
    const label = (body.firstElementChild as HTMLElement | null)?.getBoundingClientRect().width ?? 54
    const colW = (rect.width - label) / days.length
    const di = Math.min(days.length - 1, Math.max(0, Math.floor((x - rect.left - label) / colW)))
    return { day: days[di], minute: first + ((y - rect.top) / HOUR) * 60 }
  }

  const move = (e: React.PointerEvent) => {
    const g = dragRef.current
    if (!g) return
    const dx = e.clientX - g.x
    const dy = e.clientY - g.y
    const moved = g.moved || Math.hypot(dx, dy) > 6
    const at = locate(e.clientX, e.clientY)
    const near = moved && at ? nearestGuide(g.guides, at.day, at.minute - g.grabMin) : null
    if (moved && near !== g.near) onPreview?.(near)
    set({ ...g, dx, dy, moved, near })
  }

  const end = () => {
    const g = dragRef.current
    if (!g) return
    set(null)
    if (g.moved) {
      suppressClick.current = true
      window.setTimeout(() => { suppressClick.current = false }, 0)
      onPreview?.(null)
      setSpread(null)
      if (g.near && !g.near.current) onDropGuide?.(g.b, g.near)
    }
  }

  const tag = (g: Guide) => {
    const delta = baseScore == null ? null : g.score - baseScore
    return `${g.score.toFixed(1)}${delta !== null && Math.abs(delta) >= 0.05 ? ` (${delta > 0 ? '+' : ''}${delta.toFixed(1)})` : ''}`
  }

  return (
    <div className="v2-cal" role="table" aria-label="Weekly timetable" style={{ ['--days' as string]: days.length }}>
      <div className="v2-calhead" role="row">
        <div />
        {days.map((d) => <div key={d} role="columnheader">{DAY_NAMES[d].slice(0, 3)}</div>)}
      </div>
      <div className="v2-calbody" style={{ height }} ref={bodyRef} onClick={(e) => { if (e.target === e.currentTarget || (e.target as HTMLElement).classList.contains('v2-line')) setSpread(null) }}>
        <div className="v2-times" aria-hidden="true">
          {hours.map((m) => <div key={m} style={{ height: HOUR }}>{`${m / 60 > 12 ? m / 60 - 12 : m / 60} ${m / 60 >= 12 ? 'PM' : 'AM'}`}</div>)}
        </div>
        {days.map((d) => (
          <div key={d} className="v2-col" role="cell" aria-label={DAY_NAMES[d]}>
            {hours.map((m) => <div key={m} className="v2-line" style={{ height: HOUR }} />)}
            {drag?.moved ? drag.guides.flatMap((g, gi) => g.here.filter((m) => m.day === d).map((m, mi) => (
              <div key={`g${gi}-${mi}`} className={`v2-guide${g.redo ? ' is-redo' : g.clash ? ' is-clash' : ''}${g.current ? ' is-current' : ''}${g === drag.near ? ' is-near' : ''}`} style={{ top: top(m.start) + 1, height: ((m.end - m.start) / 60) * HOUR - 2 }} aria-hidden="true">
                {mi === 0 || g.here.filter((x) => x.day === d).length === 1 ? <span className="v2-guide__tag">{g.current ? 'now ' : ''}{tag(g)}{g.redo ? ' · drop + re-enrol' : ''}{g.clash ? ' · overlaps' : ''}</span> : null}
              </div>
            ))) : null}
            {drag?.moved && drag.near ? drag.near.others.filter((m) => m.day === d).map((m, i) => (
              <div key={`o${i}`} className="v2-guide v2-guide--ghost" style={{ top: top(m.start) + 1, height: ((m.end - m.start) / 60) * HOUR - 2 }} aria-hidden="true">
                <span className="v2-guide__tag">{kindShort(m.kind)} moves here</span>
              </div>
            )) : null}
            {blocks.filter((b) => b.day === d).map((b) => {
              const h = ((b.end - b.start) / 60) * HOUR
              const clash = overlaps.some((o) => involved(b, o))
              const ln = lanes.get(blockKey(b)) ?? { lane: 0, lanes: 1, cluster: '' }
              const spreadOut = ln.lanes > 1 && spread === ln.cluster
              const dragging = drag?.moved && drag.b === b
              const style: React.CSSProperties = {
                top: top(b.start) + 1,
                height: h - 2,
                zIndex: dragging ? 60 : active === b.courseCode ? 20 + ln.lane : 1 + ln.lane,
              }
              if (spreadOut) {
                style.width = `calc((100% - 6px) / ${ln.lanes})`
                style.left = `calc(3px + ${ln.lane} * (100% - 6px) / ${ln.lanes})`
                style.right = 'auto'
              } else if (ln.lanes > 1) {
                style.left = 3 + Math.min(ln.lane, 4) * INSET
              }
              if (dragging && drag) style.transform = `translate(${drag.dx}px, ${drag.dy}px)`
              const canDrag = !!guidesFor && !b.locked && !quiet
              return (
                <button
                  type="button"
                  key={blockKey(b)}
                  className={`v2-block c${colorOf(b.courseCode)}${b.locked || quiet ? ' is-locked' : ''}${b.redo ? ' is-redo' : ''}${clash ? ' is-clash' : ''}${active === b.courseCode ? ' is-active' : ''}${dragging ? ' is-dragging' : ''}${ln.lanes > 1 ? ' is-stacked' : ''}`}
                  style={style}
                  onClick={() => { if (!suppressClick.current) onPick?.(b.courseCode) }}
                  onPointerDown={(e) => {
                    if (e.button !== 0 || !canDrag) return
                    const rect = e.currentTarget.getBoundingClientRect()
                    set({ b, guides: guidesFor!(b), x: e.clientX, y: e.clientY, dx: 0, dy: 0, grabMin: ((e.clientY - rect.top) / HOUR) * 60, moved: false, near: null })
                    e.currentTarget.setPointerCapture(e.pointerId)
                  }}
                  onPointerMove={move}
                  onPointerUp={end}
                  onPointerCancel={() => { set(null); onPreview?.(null) }}
                  disabled={!onPick}
                  title={`${b.redo ? 'Only the lab changes: PeopleSoft refuses that as a swap, so it is dropped and enrolled again.\n' : ''}${b.locked ? '' : 'Drag to move. '}${b.courseCode} ${kindShort(b.kind)}\n${fmtCompact(b.start, b.end)}\n${b.instructors.join(', ')}${b.room ? `\nRoom ${b.room}` : ''}`}
                >
                  <strong>{b.locked ? '🔒 ' : ''}{b.redo ? '↻ ' : ''}{b.pinned ? '📌 ' : ''}{b.courseCode}{/^online/i.test(b.room.trim()) ? <em className="v2-online"> · Online</em> : null}</strong>
                  {h >= 40 ? <span>{kindShort(b.kind)} · {fmtCompact(b.start, b.end)}</span> : null}
                  {h >= 64 ? <span> · {b.instructors.join(', ')}</span> : null}
                </button>
              )
            })}
            {overlaps.filter((o) => o.day === d).map((o, i) => {
              const cluster = lanes.get(blockKey(o.a))?.cluster ?? ''
              return (
                <div key={i} className="v2-hatch" style={{ top: top(o.start) + 1, height: Math.max(8, ((o.end - o.start) / 60) * HOUR - 2) }}>
                  <button
                    type="button"
                    className="v2-hatch__btn"
                    title={`${o.a.courseCode} and ${o.b.courseCode} overlap here. Click to spread the cards, then pick one.`}
                    aria-label={`${o.a.courseCode} overlaps ${o.b.courseCode}: spread the cards`}
                    onClick={() => setSpread((s) => (s === cluster ? null : cluster))}
                  >
                    ⇔
                  </button>
                  {spread === cluster ? <span className="v2-hatch__ask">Which card? Click one, then drag it</span> : null}
                </div>
              )
            })}
          </div>
        ))}
      </div>
    </div>
  )
}
