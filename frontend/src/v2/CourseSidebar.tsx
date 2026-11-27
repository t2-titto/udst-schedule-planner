import { useLayoutEffect, useRef, useState, type MouseEvent } from 'react'
import { isStandaloneLabTitle, type CompatibleCandidate } from './compatible'
import { getCatalogRule } from './catalogRules'
import { optionStatusOf } from './guides'
import { effective, isLocked, type PCourse, type PlannerState } from './model'
import { tileClassFor } from './SuggestionsStrip'

export interface CompatibleStatus {
  loading: boolean
  checked: boolean
  openCount: number
  waitlistCount: number
  closedCount: number
  notOffered: boolean
}

interface Props {
  open?: boolean
  courses: PCourse[]
  state: PlannerState
  term: string
  tab?: 'current' | 'eligible'
  onTabChange?: (tab: 'current' | 'eligible') => void
  showWaitlist: boolean
  onToggleWaitlist: () => void
  showClosed: boolean
  onToggleClosed: () => void
  compatible: CompatibleCandidate[]
  compatStatus: Record<string, CompatibleStatus>
  readyLoading: boolean
  onToggleCourse: (code: string) => void
  onToggleLock: (code: string) => void
  onSelectBlockCourse: (code: string) => void
  onAddCompatible: (cand: CompatibleCandidate, withCoreq?: boolean) => void
  onInspectCourse: (code: string) => void
  onSearchCatalogFallback?: (query: string) => void
  searchingFallback?: boolean
  creditsAfter: number
  creditLimit: number
  addingCodes?: string[]
  justAddedCode?: string | null
  addNotice?: string | null
  onClearAddNotice?: () => void
  tileMap?: ReadonlyMap<string, string>
}

export function CourseSidebar(props: Props) {
  const {
    open = true,
    courses,
    state,
    showWaitlist,
    onToggleWaitlist,
    showClosed,
    onToggleClosed,
    compatible,
    compatStatus,
    readyLoading,
    onToggleCourse,
    onToggleLock,
    onSelectBlockCourse,
    onAddCompatible,
    onInspectCourse,
    onSearchCatalogFallback,
    searchingFallback,
    creditsAfter,
    creditLimit,
    addingCodes,
    justAddedCode,
    addNotice,
    onClearAddNotice,
    tileMap,
  } = props

  const [localTab, setLocalTab] = useState<'current' | 'eligible'>('current')
  const tab = props.tab ?? localTab
  const setTab = (next: 'current' | 'eligible') => {
    setLocalTab(next)
    props.onTabChange?.(next)
  }
  const [query, setQuery] = useState('')
  const asideRef = useRef<HTMLElement | null>(null)
  const prevOpenRef = useRef(open)

  // PowerPoint Morph transition when toggling Show / Hide courses
  useLayoutEffect(() => {
    const prevOpen = prevOpenRef.current
    prevOpenRef.current = open
    if (prevOpen === open) return
    const aside = asideRef.current
    if (!aside || typeof aside.animate !== 'function') return
    const easing = 'cubic-bezier(0.32, 0.72, 0, 1)'
    const cancelAll = (el: Element | null) => {
      if (el && typeof (el as HTMLElement).getAnimations === 'function') {
        for (const a of (el as HTMLElement).getAnimations()) a.cancel()
      }
    }
    const filterBox = aside.firstElementChild as HTMLElement | null
    const tabsBox = aside.querySelector<HTMLElement>('.sidebar-tabs')
    const cards = [...aside.querySelectorAll<HTMLElement>('.sidebar-course-card')]
    const vectors: Array<[number, number]> = [
      [-46, 16],
      [48, 20],
      [-26, 44],
      [38, -18],
    ]

    cancelAll(aside)
    cancelAll(filterBox)
    cancelAll(tabsBox)
    cards.forEach(cancelAll)

    if (open) {
      aside.animate(
        [
          { visibility: 'visible', opacity: 0, transform: 'translate3d(-340px, 0, 0) scale(0.95)' },
          { visibility: 'visible', opacity: 1, transform: 'translate3d(0, 0, 0) scale(1)' },
        ],
        { duration: 430, easing },
      )
      filterBox?.animate?.(
        [
          { opacity: 0, transform: 'translate3d(0, -44px, 0) scale(0.92)' },
          { opacity: 1, transform: 'translate3d(0, 0, 0) scale(1)' },
        ],
        { duration: 420, delay: 25, easing, fill: 'backwards' },
      )
      tabsBox?.animate?.(
        [
          { opacity: 0, transform: 'translate3d(56px, -18px, 0) scale(0.94)' },
          { opacity: 1, transform: 'translate3d(0, 0, 0) scale(1)' },
        ],
        { duration: 420, delay: 45, easing, fill: 'backwards' },
      )
      cards.slice(0, 8).forEach((card, i) => {
        if (typeof card.animate !== 'function') return
        const [vx, vy] = vectors[i % vectors.length]
        card.animate(
          [
            { opacity: 0, transform: `translate3d(${vx}px, ${vy}px, 0) scale(0.88)` },
            { opacity: 1, transform: 'translate3d(0, 0, 0) scale(1)' },
          ],
          { duration: 440, delay: Math.min(i * 28, 160), easing, fill: 'backwards' },
        )
      })
    } else {
      aside.animate(
        [
          { visibility: 'visible', opacity: 1, transform: 'translate3d(0, 0, 0) scale(1)' },
          { visibility: 'visible', opacity: 0, transform: 'translate3d(-340px, 0, 0) scale(0.95)' },
        ],
        { duration: 400, easing },
      )
      filterBox?.animate?.(
        [
          { opacity: 1, transform: 'translate3d(0, 0, 0) scale(1)' },
          { opacity: 0, transform: 'translate3d(-44px, -32px, 0) scale(0.94)' },
        ],
        { duration: 360, easing },
      )
      tabsBox?.animate?.(
        [
          { opacity: 1, transform: 'translate3d(0, 0, 0) scale(1)' },
          { opacity: 0, transform: 'translate3d(44px, -16px, 0) scale(0.94)' },
        ],
        { duration: 360, easing },
      )
      cards.slice(0, 8).forEach((card, i) => {
        if (typeof card.animate !== 'function') return
        const [vx, vy] = vectors[i % vectors.length]
        card.animate(
          [
            { opacity: 1, transform: 'translate3d(0, 0, 0) scale(1)' },
            { opacity: 0, transform: `translate3d(${Math.round(vx * 0.7)}px, ${vy}px, 0) scale(0.9)` },
          ],
          { duration: 360, easing },
        )
      })
    }
  }, [open])

  // Directional stagger when switching Planned <-> Ready to Add tabs
  const prevTabRef = useRef(tab)
  useLayoutEffect(() => {
    if (prevTabRef.current === tab) return
    prevTabRef.current = tab
    const aside = asideRef.current
    if (!aside || !open) return
    const cards = [...aside.querySelectorAll<HTMLElement>('.sidebar-course-card')]
    const vectors: Array<[number, number]> = [
      [-58, 12],
      [54, 16],
      [-26, 36],
      [44, -12],
    ]
    cards.slice(0, 8).forEach((card, i) => {
      if (typeof card.animate !== 'function') return
      const [vx, vy] = vectors[i % vectors.length]
      card.animate(
        [
          { opacity: 0, transform: `translate3d(${vx}px, ${vy}px, 0) scale(0.92)` },
          { opacity: 1, transform: 'translate3d(0, 0, 0) scale(1)' },
        ],
        {
          duration: 380,
          delay: Math.min(i * 24, 140),
          easing: 'cubic-bezier(0.32, 0.72, 0, 1)',
          fill: 'backwards',
        },
      )
    })
  }, [tab, open])

  const q = query.trim().toLowerCase()
  const visibleSeatsFor = (st: CompatibleStatus | undefined) => {
    if (!st) return 0
    return st.openCount + (showWaitlist ? st.waitlistCount : 0) + (showClosed ? st.closedCount : 0)
  }

  const anyChecked = compatible.some((c) => compatStatus[c.code]?.checked)
  const addableBase = compatible.filter((c) => {
    const st = compatStatus[c.code]
    // Wait for background verification when available; never advertise un-offered or zero-seat filler
    if (st?.checked) {
      if (st.notOffered || visibleSeatsFor(st) <= 0) return false
    } else if (readyLoading || anyChecked) {
      return false
    }
    // If a required corequisite is also unfinished in `compatible`, verify it is addable too
    for (const coreqCode of c.coreqs) {
      const coreqInCompat = compatible.find((x) => x.code === coreqCode)
      if (coreqInCompat) {
        const cst = compatStatus[coreqCode]
        if (cst?.checked && (cst.notOffered || visibleSeatsFor(cst) <= 0)) return false
      }
    }
    return true
  })

  const filteredAddable = (q
    ? addableBase.filter((c) => c.code.toLowerCase().includes(q) || c.title.toLowerCase().includes(q))
    : addableBase
  ).sort((a, b) => {
    const sa = compatStatus[a.code]
    const sb = compatStatus[b.code]
    const diff = visibleSeatsFor(sb) - visibleSeatsFor(sa)
    return diff !== 0 ? diff : a.code.localeCompare(b.code)
  })

  const openContext = (e: MouseEvent, code: string) => {
    e.preventDefault()
    onInspectCourse(code)
  }

  return (
    <aside ref={asideRef} className="sidebar-panel" aria-label="Courses and filters" aria-hidden={!open}>
      <div>
        <div className="sidebar-section-head">Availability filters</div>
        <div className="filter-strip">
          <button type="button" className="filter-chip on" aria-pressed="true" disabled>Open</button>
          <button type="button" className={`filter-chip ${showWaitlist ? 'on' : ''}`} aria-pressed={showWaitlist} onClick={onToggleWaitlist}>
            Waitlist
          </button>
          <button type="button" className={`filter-chip ${showClosed ? 'on' : ''}`} aria-pressed={showClosed} onClick={onToggleClosed}>
            Closed
          </button>
        </div>
      </div>

      {creditsAfter > creditLimit ? (
        <div className="status-badge warn">
          {creditsAfter} / {creditLimit} cr · over normal limit
        </div>
      ) : null}

      <div className="sidebar-tabs" role="tablist" aria-label="Course panels">
        <button
          type="button"
          role="tab"
          aria-selected={tab === 'current'}
          className={`sidebar-tab-btn ${tab === 'current' ? 'active' : ''}`}
          onClick={() => setTab('current')}
        >
          Planned ({courses.length})
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={tab === 'eligible'}
          className={`sidebar-tab-btn ${tab === 'eligible' ? 'active' : ''}`}
          onClick={() => setTab('eligible')}
        >
          Ready to Add ({addableBase.length})
        </button>
      </div>

      {addNotice ? (
        <div className="inline-popup" role="status">
          <span>{addNotice}</span>
          <div className="pill-row">
            {justAddedCode && tab !== 'current' ? (
              <button type="button" className="btn btn-sm" onClick={() => setTab('current')}>Show</button>
            ) : null}
            <button type="button" className="btn btn-sm" onClick={onClearAddNotice}>OK</button>
          </div>
        </div>
      ) : null}

      {tab === 'current' ? (
        <div className="sidebar-scroll-body">
          <div className="course-list">
            {courses.map((c) => {
              const removed = state.removed.includes(c.code)
              const locked = isLocked(c, state)
              const cur = effective(c, state)
              const curStatus = cur ? optionStatusOf(cur) : null
              const unlocksCount = getCatalogRule(c.code)?.unlocks?.length ?? 0

              return (
                <div
                  key={c.code}
                  className={`course-card sidebar-course-card has-tile ${tileClassFor(c.code, tileMap)} ${justAddedCode === c.code ? 'just-added' : ''} ${removed ? 'out' : ''}`}
                  onContextMenu={(e) => openContext(e, c.code)}
                >
                  <div className="course-card-top">
                    <div>
                      <div className="course-code">
                        <span className="course-color-dot" aria-hidden="true" />
                        {c.code}
                      </div>
                      <div className="course-title">{c.title}</div>
                    </div>
                    <button
                      type="button"
                      className="icon-btn"
                      aria-label={`Details and unlocks for ${c.code}`}
                      title="View prerequisites & what it unlocks"
                      onClick={() => onInspectCourse(c.code)}
                    >
                      ⓘ
                    </button>
                  </div>

                  <div className="course-meta-row">
                    <div className="pill-row">
                      {isStandaloneLabTitle(c.title) ? (
                        <span className="status-badge accent">Lab</span>
                      ) : cur?.components.some((cm) => /\b(lab|laboratory|tut|tutorial)\b/i.test(cm.kind)) && cur?.components.some((cm) => /\b(lec|lecture)\b/i.test(cm.kind)) ? (
                        <span className="status-badge">Lec + Lab</span>
                      ) : null}
                      {c.held ? <span className={`status-badge ${c.held.waitlisted ? 'warn' : 'ok'}`}>{c.held.waitlisted ? 'Waitlisted' : 'Enrolled'}</span> : null}
                      {c.held ? (
                        <button
                          type="button"
                          className="status-badge lock-toggle"
                          title={locked ? 'Unlock to drag or swap sections' : 'Lock section'}
                          onClick={() => onToggleLock(c.code)}
                        >
                          {locked ? '🔒 Locked' : '🔓 Swappable'}
                        </button>
                      ) : null}
                      {curStatus === 'Closed' ? <span className="status-badge bad">Full</span> : null}
                      {curStatus === 'Waitlist' && !c.held ? <span className="status-badge warn">Waitlist</span> : null}
                      {c.options.length === 0 && !c.held ? <span className="status-badge warn">Not offered</span> : null}
                      {unlocksCount > 0 ? <span className="status-badge">Unlocks {unlocksCount}</span> : null}
                    </div>
                  </div>
                  <div className="course-card-actions">
                    {!removed && c.options.length > 0 ? (
                      <button type="button" className="btn btn-sm" onClick={() => onSelectBlockCourse(c.code)}>
                        Sections
                      </button>
                    ) : null}
                    <button type="button" className="btn btn-sm" onClick={() => onToggleCourse(c.code)}>
                      {removed ? 'Restore' : c.held ? 'Drop' : 'Remove'}
                    </button>
                  </div>
                </div>
              )
            })}
          </div>
        </div>
      ) : (
        <div className="sidebar-scroll-body">
          <div className="sidebar-section-head">
            <span>Addable now ({filteredAddable.length})</span>
            {readyLoading ? (
              <span className="suggest-sub"><span className="btn-spin" aria-hidden="true" /> Checking…</span>
            ) : null}
          </div>
          <input
            type="search"
            aria-label="Filter compatible degree courses"
            placeholder="Search addable courses…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />

          <div className="course-list">
            {filteredAddable.slice(0, 30).map((cand) => {
              const st = compatStatus[cand.code]
              const isAdding = !!addingCodes?.includes(cand.code)
              const openCount = st?.openCount ?? null
              const wlCount = st?.waitlistCount ?? 0
              const clCount = st?.closedCount ?? 0
              const visibleCount = (openCount ?? 0) + (showWaitlist ? wlCount : 0) + (showClosed ? clCount : 0)
              const hasCoreq = cand.coreqs.length > 0
              const coreqCode = cand.coreqs[0]
              const coreqInList = compatible.some((x) => x.code === coreqCode) || courses.some((x) => x.code === coreqCode)

              const onTapAdd = () => {
                onAddCompatible(cand, false)
              }

              return (
                <div key={cand.code} className={`course-card sidebar-course-card has-tile ${tileClassFor(cand.code, tileMap)}`} onContextMenu={(e) => openContext(e, cand.code)}>
                  <div className="course-card-top">
                    <div>
                      <div className="course-code">
                        <span className="course-color-dot" aria-hidden="true" />
                        {cand.code}
                        {cand.units != null ? <span className="suggest-sub"> · {cand.units} cr</span> : null}
                      </div>
                      <div className="course-title">{cand.title}</div>
                    </div>
                    <button
                      type="button"
                      className="icon-btn"
                      aria-label={`Details and unlocks for ${cand.code}`}
                      onClick={() => onInspectCourse(cand.code)}
                    >
                      ⓘ
                    </button>
                  </div>

                  <div className="course-meta-row">
                    <div className="pill-row">
                      {cand.labRole === 'lab' || isStandaloneLabTitle(cand.title) ? (
                        <span className="status-badge accent">Lab</span>
                      ) : cand.labRole === 'lecture' ? (
                        <span className="status-badge">Lecture</span>
                      ) : null}
                      {cand.prereqStatus === 'check' ? (
                        <span className="status-badge warn" title={cand.prereqText}>● Check requirement</span>
                      ) : null}
                      {cand.unlocks.length > 0 ? (
                        <span className="status-badge">Unlocks {cand.unlocks.length}</span>
                      ) : null}
                      {st?.checked ? (
                        <span className="status-badge ok">{visibleCount} {visibleCount === 1 ? 'section' : 'sections'}</span>
                      ) : (
                        <span className="status-badge">Eligible</span>
                      )}
                      {hasCoreq ? <span className="status-badge">+ needs {coreqCode}</span> : null}
                      {creditsAfter + (cand.units ?? 3) > creditLimit ? (
                        <span className="status-badge warn">Over {creditLimit} cr</span>
                      ) : null}
                    </div>
                  </div>
                  <div className="course-card-actions">
                    {hasCoreq && coreqInList ? (
                      <button
                        type="button"
                        className="btn btn-sm btn-primary"
                        disabled={isAdding}
                        onClick={() => onAddCompatible(cand, true)}
                      >
                        {isAdding ? <><span className="btn-spin" aria-hidden="true" /> Adding…</> : `Add + ${coreqCode}`}
                      </button>
                    ) : (
                      <button
                        type="button"
                        className="btn btn-sm btn-primary"
                        disabled={isAdding}
                        onClick={onTapAdd}
                      >
                        {isAdding ? <><span className="btn-spin" aria-hidden="true" /> Adding…</> : '+ Add'}
                      </button>
                    )}
                  </div>
                </div>
              )
            })}

            {filteredAddable.length === 0 && readyLoading ? (
              <p className="calm-notice">Checking which eligible courses are open this term…</p>
            ) : null}

            {filteredAddable.length === 0 && !readyLoading && !q ? (
              <p className="calm-notice">No other offered, open degree courses right now. Enable Waitlist above to see waitlisted options.</p>
            ) : null}

            {filteredAddable.length === 0 && q && onSearchCatalogFallback ? (
              <div className="calm-notice">
                <p>No addable degree match for &ldquo;{query}&rdquo;.</p>
                <button
                  type="button"
                  className="btn btn-sm"
                  disabled={searchingFallback}
                  onClick={() => onSearchCatalogFallback(query)}
                >
                  {searchingFallback ? 'Searching catalog…' : 'Search full catalog'}
                </button>
              </div>
            ) : null}
          </div>
        </div>
      )}
    </aside>
  )
}
