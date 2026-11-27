import { useEffect, useLayoutEffect, useMemo, useRef, useState, type MutableRefObject, type PointerEvent } from 'react'
import { DAY_SHORT, type ClassOption, type Settings } from '../engine/types'
import { getCatalogRule } from './catalogRules'
import { isStandaloneLabTitle } from './compatible'
import {
  guidesFor,
  nearestGuide,
  optionStatusOf,
  teacherOfComp,
  type Guide,
  type GuideFilters,
  type SlotChoice,
} from './guides'
import { effective, isLocked, overlapsOf, redoChange, swappable, type PBlock, type PCourse, type PlannerState } from './model'
import { tileClassFor } from './SuggestionsStrip'

export interface Placement {
  lane: number
  lanes: number
  cluster: number
}

const blockKey = (b: PBlock) => `${b.courseCode}-${b.classNbr}-${b.day}-${b.start}`

/** Keep pure overlap-lane geometry exported for `calendar.test.ts` and multi-column positioning. */
export function lanesOf(blocks: PBlock[]): Map<string, Placement> {
  const byDay = new Map<number, PBlock[]>()
  for (const b of blocks) {
    const list = byDay.get(b.day) ?? []
    list.push(b)
    byDay.set(b.day, list)
  }
  const out = new Map<string, Placement>()
  let clusterId = 0
  for (const dayBlocks of byDay.values()) {
    const sorted = [...dayBlocks].sort((a, b) => a.start - b.start || (b.end - b.start) - (a.end - a.start))
    const clusters: PBlock[][] = []
    let cur: PBlock[] = []
    let curEnd = -1
    for (const b of sorted) {
      if (cur.length === 0 || b.start < curEnd) {
        cur.push(b)
        curEnd = Math.max(curEnd, b.end)
      } else {
        clusters.push(cur)
        cur = [b]
        curEnd = b.end
      }
    }
    if (cur.length) clusters.push(cur)

    for (const cl of clusters) {
      const id = clusterId++
      const ends: number[] = []
      const placed: { b: PBlock; lane: number }[] = []
      for (const b of cl) {
        let lane = ends.findIndex((e) => e <= b.start)
        if (lane < 0) {
          lane = ends.length
          ends.push(b.end)
        } else {
          ends[lane] = b.end
        }
        placed.push({ b, lane })
      }
      const lanes = ends.length
      for (const p of placed) out.set(blockKey(p.b), { lane: p.lane, lanes, cluster: id })
    }
  }
  return out
}

const clamp = (n: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, n))
const laneCls = (p: Placement | undefined) => {
  if (!p || p.lanes <= 1) return 'lane-0-1'
  const lanes = Math.min(p.lanes, 3)
  return `lane-${Math.min(p.lane, lanes - 1)}-${lanes}`
}

const fmtHr = (h: number) => `${h % 12 === 0 ? 12 : h % 12}${h >= 12 ? 'p' : 'a'}`
const fmtMin = (m: number) => {
  const h = Math.floor(m / 60)
  const mm = m % 60
  const ap = h >= 12 ? 'p' : 'a'
  const h12 = h % 12 === 0 ? 12 : h % 12
  return mm === 0 ? `${h12}${ap}` : `${h12}:${String(mm).padStart(2, '0')}${ap}`
}
const fmtSec = (sec: string) => sec.replace(/^sec\s*/i, '').trim()
const fmtSeats = (open: number | null, total: number | null) =>
  open != null && total != null ? `${open}/${total} seats` : open != null ? `${open} open` : ''

const isLabBlock = (b: PBlock, course?: PCourse): boolean =>
  /\b(lab|laboratory|tut|tutorial|practic|studio|workshop)\b/i.test(b.kind) || isStandaloneLabTitle(course?.title ?? '')

const isOnlineRoom = (room: string): boolean => /^online/i.test(room.trim())

interface Props {
  courses: PCourse[]
  state: PlannerState
  blocks: PBlock[]
  settings: Settings
  filters: GuideFilters
  onPickOption: (courseCode: string, optionId: string) => void
  onToggleLock: (courseCode: string) => void
  onUnlockCourse?: (courseCode: string) => void
  onToggleCourse: (courseCode: string) => void
  inspectedCourseCode: string | null
  onInspectCourse: (code: string | null) => void
  favTeachers: string[]
  onToggleFavTeacher: (name: string) => void
  onToggleAllowRedo: () => void
  sidebarOpen: boolean
  onToggleSidebar: () => void
  justAddedCourse?: string | null
  morphCaptureRef?: MutableRefObject<((mode: 'schedule' | 'layout') => void) | null>
  tileMap?: ReadonlyMap<string, string>
}

interface DragSession {
  b: PBlock
  guides: Guide[]
  startX: number
  startY: number
  grabMin: number
  moved: boolean
  near: Guide | null
  nearPrimaryIdx: number
}

function pickPrimaryIdx(guide: Guide | null, pos: { day: number; min: number } | null, grabMin: number): number {
  if (!guide || guide.here.length <= 1 || !pos) return 0
  let bestIdx = 0
  let bestDist = Infinity
  guide.here.forEach((m, idx) => {
    const d = Math.abs(m.day - pos.day) * 600 + Math.abs(m.start - (pos.min - grabMin))
    if (d < bestDist) {
      bestDist = d
      bestIdx = idx
    }
  })
  return bestIdx
}

function pickStableGuide(guides: Guide[], prev: Guide | null, day: number, start: number): Guide | null {
  const best = nearestGuide(guides, day, start)
  if (!best || !prev || prev.optionId === best.optionId) return best
  const prevInList = guides.find((g) => g.optionId === prev.optionId)
  if (!prevInList) return best
  const distOf = (g: Guide) => Math.min(...g.here.map((m) => Math.abs(m.day - day) * 600 + Math.abs(m.start - start)))
  return distOf(prevInList) - distOf(best) <= 22 ? prevInList : best
}

function readVisualTransform(el: HTMLElement): { x: number; y: number; scale: number } {
  const raw = typeof window !== 'undefined' && typeof window.getComputedStyle === 'function'
    ? window.getComputedStyle(el).transform
    : 'none'
  if (!raw || raw === 'none' || typeof DOMMatrix !== 'function') return { x: 0, y: 0, scale: 1 }
  const m = new DOMMatrix(raw)
  const scale = Math.hypot(m.a, m.b) || 1
  return { x: m.m41, y: m.m42, scale }
}

export function Calendar(props: Props) {
  const {
    courses,
    state,
    blocks,
    settings,
    filters,
    onPickOption,
    onToggleLock,
    onUnlockCourse,
    onToggleCourse,
    inspectedCourseCode,
    onInspectCourse,
    favTeachers,
    onToggleFavTeacher,
    onToggleAllowRedo,
    sidebarOpen,
    onToggleSidebar,
    justAddedCourse,
    morphCaptureRef,
    tileMap,
  } = props

  const [mobileDayMode, setMobileDayMode] = useState(false)
  const [selectedDay, setSelectedDay] = useState(0)
  const [drag, setDrag] = useState<DragSession | null>(null)
  const [dragToast, setDragToast] = useState<string | null>(null)
  const [peekBlock, setPeekBlock] = useState<PBlock | null>(null)
  const [sameTimeChoices, setSameTimeChoices] = useState<{ courseCode: string; choices: SlotChoice[] } | null>(null)
  const gridRef = useRef<HTMLDivElement | null>(null)
  const calendarCardRef = useRef<HTMLElement | null>(null)
  const dayHeadElsRef = useRef<(HTMLDivElement | null)[]>([])
  const gridRectRef = useRef<DOMRect | null>(null)
  const dragElRef = useRef<HTMLButtonElement | null>(null)
  const blockElsRef = useRef(new Map<string, HTMLButtonElement>())
  const companionAnimRef = useRef(new Map<string, Animation>())
  const companionTargetRef = useRef(new Map<string, string>())
  const pendingFlipRef = useRef<Map<string, DOMRect>>(new Map())
  const pendingDayRectsRef = useRef<(DOMRect | null)[]>([])
  const pendingCardRectRef = useRef<DOMRect | null>(null)
  const morphModeRef = useRef<'drop' | 'schedule' | 'layout'>('drop')
  const toastTimer = useRef<number | null>(null)
  const longPressTimerRef = useRef<number | null>(null)
  const longPressTriggeredRef = useRef(false)
  const justDraggedRef = useRef(false)
  const dragRef = useRef<DragSession | null>(null)

  const clearLongPress = () => {
    if (longPressTimerRef.current != null) {
      window.clearTimeout(longPressTimerRef.current)
      longPressTimerRef.current = null
    }
  }

  const flipKeyMap = useMemo(() => {
    const counts = new Map<string, number>()
    const out = new Map<string, string>()
    const ordered = [...blocks].sort((a, b) => a.day - b.day || a.start - b.start || a.classNbr.localeCompare(b.classNbr))
    for (const b of ordered) {
      const base = `${b.courseCode}:${b.kind}`
      const idx = counts.get(base) ?? 0
      counts.set(base, idx + 1)
      out.set(blockKey(b), `${base}:${idx}`)
    }
    return out
  }, [blocks])

  const captureVisualRects = (courseCode: string | null, mode: 'drop' | 'schedule' | 'layout' = 'drop') => {
    const rects = new Map<string, DOMRect>()
    for (const b of blocks) {
      if (courseCode && b.courseCode !== courseCode) continue
      const bk = blockKey(b)
      const fk = flipKeyMap.get(bk)
      const el = blockElsRef.current.get(bk)
      if (fk && el) rects.set(fk, el.getBoundingClientRect())
    }
    if (mode === 'layout') {
      pendingCardRectRef.current = calendarCardRef.current?.getBoundingClientRect() ?? null
      pendingDayRectsRef.current = dayHeadElsRef.current.map((el) => el?.getBoundingClientRect() ?? null)
    } else {
      pendingCardRectRef.current = null
      pendingDayRectsRef.current = []
    }
    morphModeRef.current = mode
    pendingFlipRef.current = rects
  }

  const captureVisualRectsForCourse = (courseCode: string) => captureVisualRects(courseCode, 'drop')

  if (morphCaptureRef) {
    morphCaptureRef.current = (mode) => captureVisualRects(null, mode)
  }

  const floatBlockTo = (bk: string, el: HTMLButtonElement, tx: number, ty: number, scale: number, reset = false) => {
    const targetKey = reset ? 'rest' : `${tx.toFixed(1)},${ty.toFixed(1)},${scale}`
    if (reset && !companionTargetRef.current.has(bk)) return
    if (!reset && companionTargetRef.current.get(bk) === targetKey) return
    const cur = readVisualTransform(el)
    const prevAnim = companionAnimRef.current.get(bk)
    if (prevAnim) prevAnim.cancel()
    if (reset) companionTargetRef.current.delete(bk)
    else companionTargetRef.current.set(bk, targetKey)
    if (typeof el.animate !== 'function') {
      if (reset) el.removeAttribute('style')
      else el.style.transform = `translate3d(${tx}px, ${ty}px, 0) scale(${scale})`
      return
    }
    if (Math.hypot(tx - cur.x, ty - cur.y) < 0.5 && Math.abs(scale - cur.scale) < 0.005) {
      companionAnimRef.current.delete(bk)
      return
    }
    const anim = el.animate(
      [
        { transform: `translate3d(${cur.x}px, ${cur.y}px, 0) scale(${cur.scale})` },
        { transform: `translate3d(${tx}px, ${ty}px, 0) scale(${scale})` },
      ],
      { duration: 165, easing: 'cubic-bezier(0.22, 1, 0.36, 1)', fill: reset ? 'none' : 'forwards' },
    )
    companionAnimRef.current.set(bk, anim)
    if (reset) {
      anim.onfinish = () => {
        if (companionAnimRef.current.get(bk) === anim) companionAnimRef.current.delete(bk)
      }
    }
  }

  const returnCompanionsSmoothly = () => {
    for (const prevBk of [...companionTargetRef.current.keys()]) {
      const el = blockElsRef.current.get(prevBk)
      if (el && el !== dragElRef.current) floatBlockTo(prevBk, el, 0, 0, 1, true)
      else companionTargetRef.current.delete(prevBk)
    }
  }

  const clearDragVisual = () => {
    if (dragElRef.current) {
      dragElRef.current.removeAttribute('style')
      dragElRef.current = null
    }
    for (const anim of companionAnimRef.current.values()) anim.cancel()
    companionAnimRef.current.clear()
    companionTargetRef.current.clear()
    for (const el of blockElsRef.current.values()) {
      el.removeAttribute('style')
    }
    gridRectRef.current = null
  }

  const setDragState = (d: DragSession | null) => {
    dragRef.current = d
    setDrag(d)
  }

  const allMins = blocks.flatMap((b) => [b.start, b.end])
  const gridStartHour = Math.max(6, Math.min(8, Math.floor((allMins.length ? Math.min(...allMins) : 480) / 60)))
  const gridEndHour = Math.min(23, Math.max(18, Math.ceil((allMins.length ? Math.max(...allMins) : 1080) / 60)))
  const gridStartMin = gridStartHour * 60
  const gridEndMin = gridEndHour * 60
  const gridSpan = Math.max(60, gridEndMin - gridStartMin)
  const hoursCount = clamp(gridEndHour - gridStartHour, 8, 17)
  const hours = Array.from({ length: hoursCount + 1 }, (_, i) => gridStartHour + i)

  const yCls = (min: number) => `py-${clamp(Math.round(((min - gridStartMin) / gridSpan) * 200), 0, 200)}`
  const hCls = (start: number, end: number) => {
    const topStep = clamp(Math.round(((start - gridStartMin) / gridSpan) * 200), 0, 198)
    const endStep = clamp(Math.round(((end - gridStartMin) / gridSpan) * 200), topStep + 2, 200)
    return `ph-${clamp(endStep - topStep, 1, 200)}`
  }

  const lanes = lanesOf(blocks)
  const { overlapByBlock, overlapPairs } = useMemo(() => {
    const byBlock = new Map<string, string[]>()
    const pairs: string[] = []
    const seenPair = new Set<string>()
    for (const ol of overlapsOf(blocks)) {
      const ka = blockKey(ol.a)
      const kb = blockKey(ol.b)
      const la = byBlock.get(ka) ?? []
      if (!la.includes(ol.b.courseCode)) la.push(ol.b.courseCode)
      byBlock.set(ka, la)
      const lb = byBlock.get(kb) ?? []
      if (!lb.includes(ol.a.courseCode)) lb.push(ol.a.courseCode)
      byBlock.set(kb, lb)
      const pKey = [ol.a.courseCode, ol.b.courseCode].sort().join(' × ')
      if (!seenPair.has(pKey)) {
        seenPair.add(pKey)
        pairs.push(pKey)
      }
    }
    return { overlapByBlock: byBlock, overlapPairs: pairs }
  }, [blocks])
  const inspectedCourse = inspectedCourseCode ? courses.find((c) => c.code === inspectedCourseCode) ?? null : null
  const inspectedCur = inspectedCourse ? effective(inspectedCourse, state) : null
  const inspectedUnlocks = inspectedCourse ? getCatalogRule(inspectedCourse.code)?.unlocks ?? [] : []

  useEffect(() => {
    if (!drag) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        justDraggedRef.current = true
        clearDragVisual()
        setDragState(null)
        window.setTimeout(() => { justDraggedRef.current = false }, 50)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [drag])

  const pointerToDayMinute = (clientX: number, clientY: number): { day: number; min: number } | null => {
    const rect = gridRectRef.current ?? gridRef.current?.getBoundingClientRect()
    if (!rect || !rect.width || !rect.height) return null
    const relX = clamp((clientX - rect.left) / rect.width, 0, 0.999)
    const relY = clamp((clientY - rect.top) / rect.height, 0, 1)
    const day = mobileDayMode ? selectedDay : Math.floor(relX * 5)
    const min = gridStartMin + relY * (gridEndMin - gridStartMin)
    return { day, min }
  }

  const commitGuideDrop = (courseCode: string, guide: Guide) => {
    captureVisualRectsForCourse(courseCode)
    if (guide.optionsAtSlot.length >= 2) {
      setSameTimeChoices({ courseCode, choices: guide.optionsAtSlot })
    } else {
      onPickOption(courseCode, guide.optionId)
    }
  }

  // Float related/companion lectures & labs smoothly during drag preview (before releasing hold)
  useLayoutEffect(() => {
    const rect = gridRectRef.current ?? gridRef.current?.getBoundingClientRect()
    if (!drag?.moved || !drag.near || drag.near.current || !rect || !rect.width || !rect.height) {
      returnCompanionsSmoothly()
      return
    }
    const g = drag.near
    const colW = rect.width / (mobileDayMode ? 1 : 5)
    const pxPerMin = rect.height / gridSpan
    const dragKey = blockKey(drag.b)
    const related = blocks.filter((b) => blockKey(b) !== dragKey && b.courseCode === drag.b.courseCode)
    const nextCompanions = new Set<string>()

    // 1. Sibling slots of the same kind (e.g., Wed Lecture when dragging Mon Lecture)
    const primaryIdx = Math.min(drag.nearPrimaryIdx, Math.max(0, g.here.length - 1))
    const siblingTargets = g.here
      .filter((_, idx) => idx !== primaryIdx)
      .sort((a, b) => a.day - b.day || a.start - b.start)
    const sameKindSiblings = related
      .filter((b) => b.kind === drag.b.kind)
      .sort((a, b) => a.day - b.day || a.start - b.start)
    const pairs: Array<{ blk: PBlock; to: { day: number; start: number } }> = []
    sameKindSiblings.forEach((blk, idx) => {
      const to = siblingTargets[idx]
      if (to) pairs.push({ blk, to })
    })

    // 2. Companion components (e.g., Lab/Tut relocating to g.others)
    const usedOthers = new Set<number>()
    for (const blk of related.filter((b) => b.kind !== drag.b.kind)) {
      const idx = g.others.findIndex((o, oi) => !usedOthers.has(oi) && o.kind === blk.kind)
      if (idx >= 0) {
        usedOthers.add(idx)
        pairs.push({ blk, to: g.others[idx] })
      }
    }

    for (const { blk, to } of pairs) {
      const bk = blockKey(blk)
      const el = blockElsRef.current.get(bk)
      if (!el) continue
      nextCompanions.add(bk)
      const tx = mobileDayMode ? 0 : (to.day - blk.day) * colW
      const ty = (to.start - blk.start) * pxPerMin
      floatBlockTo(bk, el, tx, ty, 1.02)
    }

    // Smoothly float back any previously floated companion no longer targeted
    for (const prevBk of [...companionTargetRef.current.keys()]) {
      if (!nextCompanions.has(prevBk)) {
        const el = blockElsRef.current.get(prevBk)
        if (el) floatBlockTo(prevBk, el, 0, 0, 1, true)
        else companionTargetRef.current.delete(prevBk)
      }
    }
  }, [drag?.moved, drag?.near?.optionId, drag?.nearPrimaryIdx, blocks, gridSpan, mobileDayMode])

  // Smooth PowerPoint Morph / FLIP settlement across drops, suggestion swaps, and Show/Hide courses
  useLayoutEffect(() => {
    const prevRects = pendingFlipRef.current
    const prevCardRect = pendingCardRectRef.current
    const prevDayRects = pendingDayRectsRef.current
    if (prevRects.size === 0 && !prevCardRect) return
    pendingFlipRef.current = new Map()
    pendingCardRectRef.current = null
    pendingDayRectsRef.current = []

    const mode = morphModeRef.current
    const duration = mode === 'layout' ? 440 : mode === 'schedule' ? 440 : 185
    const easing = mode === 'drop' ? 'cubic-bezier(0.22, 1, 0.36, 1)' : 'cubic-bezier(0.32, 0.72, 0, 1)'

    if (mode === 'layout' && prevCardRect && calendarCardRef.current && typeof calendarCardRef.current.animate === 'function') {
      const nextCardRect = calendarCardRef.current.getBoundingClientRect()
      const deltaLeft = prevCardRect.left - nextCardRect.left
      if (deltaLeft > 8) {
        calendarCardRef.current.animate(
          [
            { clipPath: `inset(0px 0px 0px ${Math.round(deltaLeft)}px round 16px)` },
            { clipPath: 'inset(0px 0px 0px 0px round 16px)' },
          ],
          { duration, easing },
        )
      }
    }

    if (mode === 'layout' && prevDayRects.length > 0) {
      dayHeadElsRef.current.forEach((headEl, idx) => {
        const prevD = prevDayRects[idx]
        if (!headEl || !prevD || typeof headEl.animate !== 'function') return
        const nextD = headEl.getBoundingClientRect()
        const dx = prevD.left - nextD.left + (prevD.width - nextD.width) * 0.5
        if (Math.abs(dx) < 2) return
        headEl.animate(
          [{ transform: `translate3d(${dx}px, 0, 0)` }, { transform: 'translate3d(0px, 0, 0)' }],
          { duration, delay: Math.min(idx * 16, 64), easing, fill: idx > 0 ? 'backwards' : 'none' },
        )
      })
    }

    for (const b of blocks) {
      const bk = blockKey(b)
      const fk = flipKeyMap.get(bk)
      const el = blockElsRef.current.get(bk)
      if (!el || typeof el.animate !== 'function') continue
      if (typeof el.getAnimations === 'function') {
        for (const a of el.getAnimations()) a.cancel()
      }
      const prev = fk ? prevRects.get(fk) : undefined
      if (prev) {
        const next = el.getBoundingClientRect()
        const dx = prev.left - next.left
        const dy = prev.top - next.top
        const sx = prev.width / Math.max(1, next.width)
        const sy = prev.height / Math.max(1, next.height)
        const moved = Math.hypot(dx, dy) >= 2
        const resized = Math.abs(sx - 1) > 0.02 || Math.abs(sy - 1) > 0.02
        if (!moved && !resized) continue
        const delay = mode === 'drop' ? 0 : Math.min(b.day * 16, 64)
        el.animate(
          mode === 'schedule' && moved
            ? [
                { transformOrigin: 'top left', transform: `translate3d(${dx}px, ${dy}px, 0) scale(${sx}, ${sy})`, zIndex: '35' },
                { transformOrigin: 'top left', transform: `translate3d(${dx * 0.42}px, ${dy * 0.42 - 4}px, 0) scale(${1 + (sx - 1) * 0.42}, ${1 + (sy - 1) * 0.42})`, boxShadow: '0 14px 28px rgba(15, 23, 42, 0.2)', zIndex: '35', offset: 0.48 },
                { transformOrigin: 'top left', transform: 'translate3d(0px, 0px, 0) scale(1, 1)', zIndex: '35' },
              ]
            : [
                { transformOrigin: 'top left', transform: `translate3d(${dx}px, ${dy}px, 0) scale(${sx}, ${sy})` },
                { transformOrigin: 'top left', transform: 'translate3d(0px, 0px, 0) scale(1, 1)' },
              ],
          { duration, delay, easing, fill: delay > 0 ? 'backwards' : 'none' },
        )
      } else if (mode === 'schedule') {
        const edgeX = b.day <= 1 ? -46 : b.day >= 3 ? 46 : 0
        el.animate(
          [
            { opacity: 0, transform: `translate3d(${edgeX}px, 28px, 0) scale(0.88)` },
            { opacity: 1, transform: 'translate3d(0px, 0px, 0) scale(1)' },
          ],
          { duration: 430, delay: Math.min(b.day * 22, 88), easing, fill: 'backwards' },
        )
      }
    }
  }, [blocks, flipKeyMap, sidebarOpen])

  const onBlockPointerDown = (e: PointerEvent<HTMLButtonElement>, b: PBlock) => {
    if (e.button !== 0 && e.pointerType === 'mouse') return
    clearLongPress()
    longPressTriggeredRef.current = false
    if (e.pointerType === 'touch') {
      longPressTimerRef.current = window.setTimeout(() => {
        longPressTriggeredRef.current = true
        setPeekBlock(b)
      }, 360)
    }
    const c = courses.find((x) => x.code === b.courseCode)
    if (!c) return
    let effState = state
    if (isLocked(c, state)) {
      if (onUnlockCourse) onUnlockCourse(c.code)
      effState = { ...state, unlocked: [...state.unlocked, c.code] }
      if (toastTimer.current != null) window.clearTimeout(toastTimer.current)
      setDragToast(`Unlocked ${c.code} to swap`)
      toastTimer.current = window.setTimeout(() => setDragToast(null), 1800)
    }
    const guides = guidesFor(courses, effState, b, settings, filters)
    const rect = e.currentTarget.getBoundingClientRect()
    gridRectRef.current = gridRef.current?.getBoundingClientRect() ?? null
    dragElRef.current = e.currentTarget
    const grabMin = rect.height > 0 ? ((e.clientY - rect.top) / rect.height) * (b.end - b.start) : 0
    try { e.currentTarget.setPointerCapture?.(e.pointerId) } catch { /* jsdom fallback */ }
    setDragState({
      b,
      guides,
      startX: e.clientX,
      startY: e.clientY,
      grabMin,
      moved: false,
      near: null,
      nearPrimaryIdx: 0,
    })
  }

  const onBlockPointerMove = (e: PointerEvent<HTMLButtonElement>) => {
    const g = dragRef.current
    if (!g) return
    const dx = e.clientX - g.startX
    const dy = e.clientY - g.startY
    const moved = g.moved || Math.hypot(dx, dy) > 5
    if (moved) {
      clearLongPress()
      setPeekBlock(null)
    }
    if (moved && dragElRef.current) {
      dragElRef.current.style.transform = `translate3d(${dx}px, ${dy}px, 0) scale(1.03)`
    }
    const pos = moved ? pointerToDayMinute(e.clientX, e.clientY) : null
    const near = moved && pos && g.guides.length ? pickStableGuide(g.guides, g.near, pos.day, pos.min - g.grabMin) : null
    const nearPrimaryIdx = pickPrimaryIdx(near, pos, g.grabMin)
    if (moved !== g.moved || near?.optionId !== g.near?.optionId || nearPrimaryIdx !== g.nearPrimaryIdx) {
      setDragState({ ...g, moved, near, nearPrimaryIdx })
    }
  }

  const onBlockPointerUp = () => {
    clearLongPress()
    const g = dragRef.current
    const dragEl = dragElRef.current
    const dragCur = dragEl && g?.moved && (!g.near || g.near.current) ? readVisualTransform(dragEl) : null
    if (g?.moved && g.near && !g.near.current) {
      captureVisualRectsForCourse(g.b.courseCode)
    }
    clearDragVisual()
    if (dragEl && dragCur && typeof dragEl.animate === 'function' && (Math.hypot(dragCur.x, dragCur.y) >= 2)) {
      dragEl.animate(
        [
          { transform: `translate3d(${dragCur.x}px, ${dragCur.y}px, 0) scale(${dragCur.scale})` },
          { transform: 'translate3d(0px, 0px, 0) scale(1)' },
        ],
        { duration: 165, easing: 'cubic-bezier(0.22, 1, 0.36, 1)' },
      )
    }
    if (!g) return
    setDragState(null)
    if (g.moved) {
      justDraggedRef.current = true
      window.setTimeout(() => { justDraggedRef.current = false }, 50)
      if (g.near && !g.near.current) {
        commitGuideDrop(g.b.courseCode, g.near)
      }
    }
  }

  return (
    <section ref={calendarCardRef} className="calendar-card" aria-label="Weekly timetable">
      <div className="calendar-toolbar">
        <div className="pill-row">
          <button type="button" className="btn btn-sm desktop-only-toggle" onClick={onToggleSidebar}>
            {sidebarOpen ? 'Hide courses' : 'Show courses'}
          </button>
          {drag?.moved ? (
            drag.near ? (
              <span className={`status-badge ${drag.near.clash ? 'bad' : 'accent'}`}>
                {drag.near.current
                  ? `Keeping Sec ${fmtSec(drag.near.section)}`
                  : drag.near.clash && drag.near.clashesWith.length
                    ? `Sec ${fmtSec(drag.near.section)} overlaps ${drag.near.clashesWith.join(', ')}`
                    : `Drop → Sec ${fmtSec(drag.near.section)} · ${drag.near.teacher}${drag.near.others.length ? ` (+${drag.near.others[0].kind} Sec ${fmtSec(drag.near.others[0].section)})` : ''}`}
              </span>
            ) : (
              <span className="status-badge">Move onto a dashed slot</span>
            )
          ) : peekBlock ? (() => {
            const pCourse = courses.find((c) => c.code === peekBlock.courseCode)
            const pOpt = pCourse ? effective(pCourse, state) : null
            const pComp = pOpt?.components.find((cm) => cm.classNbr === peekBlock.classNbr)
            const pLab = isLabBlock(peekBlock, pCourse)
            const pOnline = isOnlineRoom(peekBlock.room)
            const seatsStr = pComp ? fmtSeats(pComp.seatsOpen, pComp.seatsTotal) : ''
            const secStr = pComp?.section ? `Sec ${fmtSec(pComp.section)}` : ''
            const pOverlaps = overlapByBlock.get(blockKey(peekBlock)) ?? []
            return (
              <span className={`status-badge cal-peek-pill ${pOverlaps.length ? 'bad' : ''}`} role="status">
                {peekBlock.courseCode} ({pCourse?.title || peekBlock.kind}) · {pLab ? 'Lab' : 'Lecture'}
                {secStr ? ` · ${secStr}` : ''} · {DAY_SHORT[peekBlock.day]} {fmtMin(peekBlock.start)}–{fmtMin(peekBlock.end)}
                {pOnline ? ' · Online' : peekBlock.room ? ` · ${peekBlock.room}` : ''}
                {peekBlock.instructors[0] ? ` · ${peekBlock.instructors[0]}` : ''}
                {seatsStr ? ` · ${seatsStr}` : ''}
                {pOverlaps.length ? ` · Overriding with ${pOverlaps.join(', ')}` : ''}
              </span>
            )
          })() : overlapPairs.length > 0 ? (
            <span className="status-badge bad" role="alert">
              Overriding: {overlapPairs.join(' · ')}
            </span>
          ) : (
            <span className="suggest-sub">Drag to swap (hold on touch) · tap for sections</span>
          )}
          {dragToast ? <span className="status-badge accent">{dragToast}</span> : null}
        </div>
        <div className="mobile-day-bar">
          <div className="mobile-mode-row">
            <button type="button" className={`choice-pill ${!mobileDayMode ? 'active' : ''}`} onClick={() => setMobileDayMode(false)}>
              Week
            </button>
            <button type="button" className={`choice-pill ${mobileDayMode ? 'active' : ''}`} onClick={() => setMobileDayMode(true)}>
              Day
            </button>
          </div>
          {mobileDayMode ? (
            <div className="mobile-days-row">
              {[0, 1, 2, 3, 4].map((d) => (
                <button key={d} type="button" className={`choice-pill ${selectedDay === d ? 'active' : ''}`} onClick={() => setSelectedDay(d)}>
                  {DAY_SHORT[d]}
                </button>
              ))}
            </div>
          ) : null}
        </div>
      </div>

      <div className={`cal-wrap ${drag ? 'dragging' : ''}`}>
        <div className="cal-hours" aria-hidden="true">
          {hours.map((h) => (
            <div key={h} className={`cal-hour-label ${yCls(h * 60)}`}>{fmtHr(h)}</div>
          ))}
        </div>

        <div className="cal-board">
          <div className={`cal-day-headers ${mobileDayMode ? 'day-mode' : ''}`}>
            {[0, 1, 2, 3, 4].map((day) => (
              <div
                key={day}
                ref={(el) => { dayHeadElsRef.current[day] = el }}
                className={`cal-day-head ${mobileDayMode && day !== selectedDay ? 'hidden-mobile' : ''}`}
              >
                {DAY_SHORT[day]}
              </div>
            ))}
          </div>

          <div ref={gridRef} className={`cal-grid-cols hours-${hoursCount} ${mobileDayMode ? 'day-mode' : ''}`}>
            {[0, 1, 2, 3, 4].map((day) => {
              const dayBlocks = blocks.filter((b) => b.day === day)
              const dayGuides = drag?.moved ? drag.guides.flatMap((g) => g.here.filter((m) => m.day === day).map((m) => ({ g, m }))) : []
              const dayGhosts = drag?.moved && drag.near && !drag.near.current
                ? drag.near.others.filter((m) => m.day === day)
                : []

              return (
                <div key={day} className={`cal-day-lane ${mobileDayMode && day !== selectedDay ? 'hidden-mobile' : ''}`}>
                  {/* Minimalistic non-interactive destination guides */}
                  {dayGuides.map(({ g, m }, idx) => {
                    const isActive = drag?.near?.optionId === g.optionId
                    const seatsLabel = g.clash && g.clashesWith.length
                      ? `Clashes w/ ${g.clashesWith[0]}`
                      : g.status === 'Closed'
                        ? 'Full'
                        : fmtSeats(g.seatsOpen, g.seatsTotal)
                    return (
                      <div
                        key={`${g.optionId}-${idx}`}
                        aria-hidden="true"
                        className={`cal-guide ${yCls(m.start)} ${hCls(m.start, m.end)} ${g.current ? 'is-current' : ''} ${isActive ? 'active' : ''} ${g.clash ? 'clash' : ''} ${g.status === 'Closed' ? 'closed' : ''}`}
                      >
                        <div className="cal-guide-teacher">
                          {g.current ? 'Now · ' : ''}
                          {g.optionsAtSlot.length >= 2 ? `${g.optionsAtSlot.length} teachers` : g.teacher}
                        </div>
                        <div className="cal-guide-sub">
                          <span>Sec {fmtSec(g.section)}</span>
                          <span>{seatsLabel}</span>
                        </div>
                      </div>
                    )
                  })}

                  {/* Companion Labs / Tutorials that move together */}
                  {dayGhosts.map((gh, idx) => (
                    <div key={`ghost-${idx}`} className={`cal-linked-ghost ${yCls(gh.start)} ${hCls(gh.start, gh.end)}`} aria-hidden="true">
                      <div>{gh.kind} moves here · Sec {fmtSec(gh.section)}</div>
                      <div className="suggest-sub">{gh.teacher}</div>
                    </div>
                  ))}

                  {dayBlocks.map((b) => {
                    const bk = blockKey(b)
                    const course = courses.find((c) => c.code === b.courseCode)
                    const opt = course ? effective(course, state) : null
                    const isClosedPicked = opt ? optionStatusOf(opt) === 'Closed' : false
                    const isDraggingThis = !!drag?.moved && drag.b === b
                    const isMovingCompanion = !!drag?.moved && drag.b !== b && drag.b.courseCode === b.courseCode
                    const isCompanionFloating = isMovingCompanion && !!drag?.near && !drag.near.current
                    const isClashTarget = !!drag?.moved && !!drag.near && !drag.near.current && drag.near.clashesWith.includes(b.courseCode)
                    const isJustAdded = justAddedCourse === b.courseCode
                    const pl = lanes.get(bk)
                    const comp = opt?.components.find((cm) => cm.classNbr === b.classNbr)
                    const isLab = isLabBlock(b, course)
                    const isOnline = isOnlineRoom(b.room)
                    const overlappingWith = overlapByBlock.get(bk) ?? []
                    const isOverlapping = overlappingWith.length > 0

                    return (
                      <button
                        key={bk}
                        ref={(el) => {
                          if (el) blockElsRef.current.set(bk, el)
                          else blockElsRef.current.delete(bk)
                        }}
                        type="button"
                        className={`cal-block ${tileClassFor(b.courseCode, tileMap)} ${isLab ? 'is-lab' : 'is-lecture'} ${isOnline ? 'is-online' : ''} ${yCls(b.start)} ${hCls(b.start, b.end)} ${laneCls(pl)} ${b.locked ? 'locked' : ''} ${isDraggingThis ? 'is-dragging' : ''} ${isCompanionFloating ? 'is-companion-floating' : isMovingCompanion ? 'dragging-source' : ''} ${isClashTarget ? 'is-clash-target' : ''} ${isOverlapping ? 'is-overlapping' : ''} ${isClosedPicked ? 'closed-picked' : ''} ${isJustAdded ? 'just-added' : ''}`}
                        onMouseEnter={() => setPeekBlock(b)}
                        onMouseLeave={() => setPeekBlock((cur) => (cur && blockKey(cur) === bk ? null : cur))}
                        onPointerDown={(e) => onBlockPointerDown(e, b)}
                        onPointerMove={onBlockPointerMove}
                        onPointerUp={onBlockPointerUp}
                        onLostPointerCapture={() => { if (dragRef.current) onBlockPointerUp() }}
                        onPointerCancel={() => { clearLongPress(); clearDragVisual(); setDragState(null) }}
                        onClick={() => {
                          if (longPressTriggeredRef.current) {
                            longPressTriggeredRef.current = false
                            return
                          }
                          if (!justDraggedRef.current) onInspectCourse(b.courseCode)
                        }}
                        onContextMenu={(e) => { e.preventDefault(); onInspectCourse(b.courseCode) }}
                      >
                        <div className="cal-block-head">
                          <span>{b.courseCode}</span>
                          <span className={`block-kind-pill ${isLab ? 'lab' : 'lec'}`}>
                            {isLab ? 'LAB' : 'LEC'}
                          </span>
                        </div>
                        <div className="cal-block-meta">
                          {isMovingCompanion
                            ? `${b.kind} moves with swap`
                            : `${b.locked ? 'Locked · ' : ''}${comp?.section ? `Sec ${fmtSec(comp.section)} · ` : ''}${isOnline ? 'Online' : (b.instructors[0] || b.room || b.kind)}`}
                        </div>
                        {isOverlapping ? (
                          <div className="cal-overlap-tag">Overlaps {overlappingWith.join(', ')}</div>
                        ) : null}
                      </button>
                    )
                  })}
                </div>
              )
            })}
          </div>
        </div>
      </div>

      {/* Same-time multi-instructor collision picker on drop */}
      {sameTimeChoices ? (
        <div className="sheet-backdrop" role="dialog" aria-modal="true" aria-label="Choose instructor for this time slot" onClick={() => setSameTimeChoices(null)}>
          <div className="sheet-card" onClick={(e) => e.stopPropagation()}>
            <div className="sheet-head">
              <div>
                <h3>Pick a teacher for {sameTimeChoices.courseCode}</h3>
                <p className="suggest-sub">Multiple sections meet at this exact time.</p>
              </div>
              <button type="button" className="btn btn-sm" onClick={() => setSameTimeChoices(null)}>Cancel</button>
            </div>
            <div className="option-list">
              {sameTimeChoices.choices.map((ch) => (
                <button
                  key={ch.option.id}
                  type="button"
                  className={`option-row ${ch.status === 'Closed' ? 'dimmed' : ''}`}
                  onClick={() => {
                    captureVisualRectsForCourse(sameTimeChoices.courseCode)
                    onPickOption(sameTimeChoices.courseCode, ch.option.id)
                    setSameTimeChoices(null)
                  }}
                >
                  <div>
                    <strong>{ch.teacher}</strong>
                    <div className="suggest-sub">Sec {ch.section} · {ch.status === 'Closed' ? 'Full' : fmtSeats(ch.seatsOpen, ch.seatsTotal)}</div>
                  </div>
                  <span className={`status-badge ${ch.status === 'Open' ? 'ok' : ch.status === 'Waitlist' ? 'warn' : 'bad'}`}>
                    {ch.status === 'Closed' ? 'Full' : ch.status}
                  </span>
                </button>
              ))}
            </div>
          </div>
        </div>
      ) : null}

      {/* Section Inspector & Swap Bottom Sheet / Modal */}
      {inspectedCourse ? (
        <div className="sheet-backdrop" role="dialog" aria-modal="true" aria-label={`${inspectedCourse.code} sections`} onClick={() => onInspectCourse(null)}>
          <div className="sheet-card" onClick={(e) => e.stopPropagation()}>
            <div className="sheet-head">
              <div>
                <h3>{inspectedCourse.code} · {inspectedCourse.title}</h3>
                <p className="suggest-sub">
                  Unlocks: {inspectedUnlocks.length ? inspectedUnlocks.join(', ') : 'No further courses depend on this'}
                </p>
              </div>
              <button type="button" className="btn btn-sm" onClick={() => onInspectCourse(null)}>Done</button>
            </div>

            <div className="course-meta-row">
              {inspectedCourse.held ? (
                <button type="button" className="btn btn-sm" onClick={() => onToggleLock(inspectedCourse.code)}>
                  {isLocked(inspectedCourse, state) ? '🔒 Locked · Tap to unlock swaps' : '🔓 Unlocked · Tap to lock'}
                </button>
              ) : null}
              <label
                className="filter-chip"
                title="Enrolled courses changing only a lab or tutorial cannot use PeopleSoft Swap — they must drop and re-enrol, risking the held seat if the section fills."
              >
                <input type="checkbox" checked={filters.allowRedo} onChange={onToggleAllowRedo} />
                <span>Allow lab-only changes (drops &amp; re-enrols · seat risk)</span>
              </label>
              <button type="button" className="btn btn-sm" onClick={() => { onToggleCourse(inspectedCourse.code); onInspectCourse(null) }}>
                {inspectedCourse.held ? 'Drop course' : 'Remove from schedule'}
              </button>
            </div>

            <div className="option-list">
              {inspectedCourse.options
                .filter((o) => {
                  const isCur = o.id === inspectedCur?.id
                  const st = optionStatusOf(o)
                  if (!isCur) {
                    if (st === 'Open' && !filters.open) return false
                    if (st === 'Waitlist' && !filters.waitlist) return false
                    if (st === 'Closed' && !filters.closed) return false
                  }
                  const redo = redoChange(inspectedCourse, o)
                  if (!swappable(inspectedCourse, o) && (!redo || !filters.allowRedo)) return false
                  return true
                })
                .map((o: ClassOption) => {
                  const mainComp = o.components[0]
                  const teacher = teacherOfComp(mainComp)
                  const st = optionStatusOf(o)
                  const isCur = o.id === inspectedCur?.id
                  const redo = redoChange(inspectedCourse, o)
                  const isFav = favTeachers.some((t) => t.toLowerCase() === teacher.toLowerCase())

                  return (
                    <div key={o.id} className={`option-row ${isCur ? 'active' : ''} ${st === 'Closed' ? 'dimmed' : ''}`}>
                      <button
                        type="button"
                        className="btn btn-sm btn-ghost"
                        onClick={() => { onPickOption(inspectedCourse.code, o.id); onInspectCourse(null) }}
                      >
                        <div>
                          <strong>{teacher} · Sec {mainComp?.section ?? o.optionNo}</strong>
                          <div className="suggest-sub">
                            {o.components.map((cm) => `${cm.kind} ${cm.section} (${fmtSeats(cm.seatsOpen, cm.seatsTotal)})`).join(' + ')}
                            {redo ? ' · drops & re-enrols' : ''}
                          </div>
                        </div>
                      </button>
                      <div className="pill-row">
                        <span className={`status-badge ${st === 'Open' ? 'ok' : st === 'Waitlist' ? 'warn' : 'bad'}`}>
                          {st === 'Closed' ? 'Full' : st}
                        </span>
                        <button
                          type="button"
                          className={`fav-heart ${isFav ? 'on' : ''}`}
                          aria-label={`Favorite ${teacher}`}
                          onClick={() => onToggleFavTeacher(teacher)}
                        >
                          {isFav ? '♥' : '♡'}
                        </button>
                      </div>
                    </div>
                  )
                })}
            </div>
          </div>
        </div>
      ) : null}
    </section>
  )
}
