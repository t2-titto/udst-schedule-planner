import catalogKeysRaw from './catalogKeys.json'
import { evalRuleTree, formatRuleTree, getCatalogRule, isPlacementRule, type RuleNode } from './catalogRules'
import { termOrder } from './data'
import type { DegreeCourse, DegreeView, WireDegree } from './degree'
import { normCode } from './prereq'
import { termCode } from './ready'

export interface CatalogKeyRecord {
  crse_id: string
  offer_nbr: string
  effdt: string
}

const CATALOG_KEYS = catalogKeysRaw as Record<string, CatalogKeyRecord>

export function lookupCatalogKey(code: string): CatalogKeyRecord | null {
  const trimmed = code.trim().toUpperCase().replace(/\s+/g, ' ')
  if (CATALOG_KEYS[trimmed]) return CATALOG_KEYS[trimmed]
  const compact = normCode(trimmed)
  for (const [k, v] of Object.entries(CATALOG_KEYS)) {
    if (normCode(k) === compact) return v
  }
  return null
}

/** Completed courses (not merely in progress) plus sum of completed credits from `degree_tree`. */
export function completionFromDegree(wire: WireDegree): { doneCodes: Set<string>; earnedCredits: number } {
  const doneCodes = new Set<string>()
  let earnedCredits = 0
  for (const r of wire.rows) {
    const code = normCode(r.code ?? '')
    if (!code) continue
    // A row with `term` non-empty and NO `term_choice` is either historical (completed) or currently enrolled;
    // Completed semesters in PeopleSoft omit from currently enrolled and appear under completed summary / past terms.
    // Specifically, `status` on course rows without term_choice and earlier term than openTerm, or we check rows:
    if (r.term && !r.term_choice) {
      // Callers can pass exact active/current codes to exclude from `doneCodes`; here record all term-assigned non-planned rows first.
      doneCodes.add(code)
      const u = Number.parseFloat(String(r.units ?? ''))
      if (Number.isFinite(u)) earnedCredits += u
    }
  }
  return { doneCodes, earnedCredits }
}

export function completionFromView(view: DegreeView, openTerm?: string): { doneCodes: Set<string>; earnedCredits: number } {
  const doneCodes = new Set<string>()
  let earnedCredits = 0
  const openRank = openTerm ? termOrder(openTerm) : 0
  for (const y of view.years) {
    for (const s of y.semesters) {
      for (const b of s.blocks) {
        if (b.kind !== 'course') continue
        const priorInProgress =
          b.course.status === 'inProgress' &&
          !!openTerm &&
          !!b.course.term &&
          termOrder(b.course.term) < openRank
        if (b.course.status === 'completed' || priorInProgress) {
          doneCodes.add(normCode(b.course.code))
          if (typeof b.course.units === 'number' && Number.isFinite(b.course.units)) {
            earnedCredits += b.course.units
          }
        }
      }
    }
  }
  return { doneCodes, earnedCredits }
}

export type PrereqStatus = 'met' | 'unmet' | 'check'

export const isStandaloneLabTitle = (title: string): boolean => /\(\s*lab(?:oratory)?\s*\)\s*$/i.test(title.trim())

export const baseTitleOfLab = (title: string): string =>
  title.trim().replace(/\s*\(\s*lab(?:oratory)?\s*\)\s*$/i, '').trim().toLowerCase()

export interface CompatibleCandidate {
  code: string
  title: string
  units: number | null
  plannedTerm: string | null
  course: DegreeCourse
  prereqStatus: PrereqStatus
  prereqText: string
  coreqs: string[]
  unlocks: string[]
  key: CatalogKeyRecord | null
  strm: string | null
  labRole?: 'lecture' | 'lab'
  pairedCode?: string
}

export function evaluateCoursePrereqs(
  code: string,
  doneCodes: Set<string>,
  earnedCredits: number,
  extraRule?: { prereqText?: string; coreqs?: string[] },
): { status: PrereqStatus; text: string; coreqs: string[]; unlocks: string[] } {
  const cat = getCatalogRule(code)
  const unlocks = cat?.unlocks ?? []
  const coreqs = [...new Set([...(cat?.corequisites ?? []), ...(extraRule?.coreqs ?? [])])]

  if (!cat) {
    if (extraRule?.prereqText) {
      return { status: 'check', text: extraRule.prereqText, coreqs, unlocks }
    }
    return { status: 'met', text: '', coreqs, unlocks }
  }

  let status: PrereqStatus = 'met'
  const parts: string[] = []

  if (cat.prerequisites) {
    const res = evalRuleTree(cat.prerequisites, doneCodes, earnedCredits)
    const text = formatRuleTree(cat.prerequisites)
    if (text) parts.push(text)
    if (res === false) status = 'unmet'
    else if (res === null || isPlacementRule(cat.prerequisites)) status = 'check'
  }

  if (cat.cohort_rules?.length) {
    const anyMet = cat.cohort_rules.some((cr) => evalRuleTree(cr.prerequisites, doneCodes, earnedCredits) === true)
    const anyNull = cat.cohort_rules.some((cr) => evalRuleTree(cr.prerequisites, doneCodes, earnedCredits) === null)
    const cohortTxt = cat.cohort_rules.map((cr) => `${cr.condition}: ${formatRuleTree(cr.prerequisites)}`).join(' · ')
    if (cohortTxt) parts.push(cohortTxt)
    if (status === 'met') {
      if (anyMet) status = 'met'
      else if (anyNull) status = 'check'
      else status = 'unmet'
    }
  }

  return { status, text: parts.join(' | '), coreqs, unlocks }
}

function nodeHasExplicitDone(node: RuleNode | null | undefined, explicitDone: Set<string>): boolean {
  if (!node || typeof node === 'object' && 'note' in node) return false
  if (typeof node === 'string') return explicitDone.has(normCode(node))
  if (node.op === 'AND') return node.items.every((it) => nodeHasExplicitDone(it, explicitDone))
  return node.items.some((it) => nodeHasExplicitDone(it, explicitDone))
}

function collectBypassedNodes(
  node: RuleNode | null | undefined,
  sourceCourse: string,
  explicitDone: Set<string>,
  out: Map<string, string>,
  queue: string[],
): void {
  if (!node || (typeof node === 'object' && 'note' in node)) return
  if (typeof node === 'string') {
    const code = normCode(node)
    if (code && code !== sourceCourse && !explicitDone.has(code) && !out.has(code)) {
      out.set(code, sourceCourse)
      queue.push(code)
    }
    return
  }
  if (node.op === 'AND') {
    for (const it of node.items) collectBypassedNodes(it, sourceCourse, explicitDone, out, queue)
    return
  }
  // For an OR branch: only treat branches as exempted when NONE of them is already completed
  if (!node.items.some((it) => nodeHasExplicitDone(it, explicitDone))) {
    for (const it of node.items) collectBypassedNodes(it, sourceCourse, explicitDone, out, queue)
  }
}

/**
 * Infer prerequisite & antirequisite courses already satisfied by exemption/bypass:
 * e.g. if the student completed or is taking MATH 1020 without MATH 1010, MATH 1010 is treated as
 * exempted (satisfied by MATH 1020) and omitted from Ready to Add unless marked for retake.
 */
export function inferExemptedCourses(
  doneCodes: Set<string>,
  alsoTaking?: Iterable<string>,
): Map<string, string> {
  const out = new Map<string, string>()
  const explicit = new Set<string>([...doneCodes].map((c) => normCode(c)))
  if (alsoTaking) {
    for (const c of alsoTaking) {
      const nc = normCode(c)
      if (nc) explicit.add(nc)
    }
  }
  const queue = [...explicit]
  const visited = new Set<string>()
  const rootOf = new Map<string, string>()
  for (const c of queue) rootOf.set(c, c)

  while (queue.length > 0) {
    const cur = queue.shift()!
    if (visited.has(cur)) continue
    visited.add(cur)
    const root = rootOf.get(cur) ?? cur
    const rule = getCatalogRule(cur)
    if (!rule) continue
    const spawned: string[] = []
    if (rule.prerequisites) {
      collectBypassedNodes(rule.prerequisites, root, explicit, out, spawned)
    }
    if (rule.antirequisites) {
      for (const anti of rule.antirequisites) {
        const na = normCode(anti)
        if (na && na !== root && !explicit.has(na) && !out.has(na)) {
          out.set(na, root)
          spawned.push(na)
        }
      }
    }
    for (const next of spawned) {
      if (!rootOf.has(next)) rootOf.set(next, root)
      queue.push(next)
    }
  }
  return out
}

/**
 * Scan every unfinished, non-active course in the student's own degree against prerequisite rules
 * (`catalogRules.json` + earned credits from `degree_tree`) and attach static `catalogKeys.json` lookup info.
 */
export function discoverCompatibleCandidates(
  view: DegreeView,
  openTerm: string,
  inSchedule: Iterable<string>,
  retakeCodes?: Iterable<string>,
): {
  eligible: CompatibleCandidate[]
  locked: CompatibleCandidate[]
  doneCodes: Set<string>
  earnedCredits: number
  exempted: Map<string, string>
} {
  const { doneCodes: rawDone, earnedCredits } = completionFromView(view, openTerm)
  const retakeSet = new Set([...(retakeCodes ?? [])].map((c) => normCode(c)))
  const exempted = inferExemptedCourses(rawDone, inSchedule)
  const doneCodes = new Set<string>([...rawDone, ...exempted.keys()])
  const have = new Set([...inSchedule].map((c) => normCode(c)))
  const strm = termCode(openTerm)
  const eligible: CompatibleCandidate[] = []
  const locked: CompatibleCandidate[] = []

  // Discover standalone Course Title <-> Course Title (Lab) pairs dynamically across the degree view
  const titleMap = new Map<string, DegreeCourse>()
  const allDegreeCourses: DegreeCourse[] = []
  for (const y of view.years) {
    for (const s of y.semesters) {
      for (const b of s.blocks) {
        if (b.kind !== 'course') continue
        allDegreeCourses.push(b.course)
        if (!isStandaloneLabTitle(b.course.title) && b.course.title.trim()) {
          titleMap.set(b.course.title.trim().toLowerCase(), b.course)
        }
      }
    }
  }
  const pairedTitleByCode = new Map<string, { role: 'lecture' | 'lab'; pairedCode: string }>()
  for (const dc of allDegreeCourses) {
    if (!isStandaloneLabTitle(dc.title)) continue
    const base = titleMap.get(baseTitleOfLab(dc.title))
    if (base && normCode(base.code) !== normCode(dc.code)) {
      pairedTitleByCode.set(normCode(dc.code), { role: 'lab', pairedCode: base.code })
      pairedTitleByCode.set(normCode(base.code), { role: 'lecture', pairedCode: dc.code })
    }
  }

  for (const y of view.years) {
    for (const s of y.semesters) {
      for (const b of s.blocks) {
        if (b.kind !== 'course') continue
        const c = b.course
        const nc = normCode(c.code)
        const isRetake = retakeSet.has(nc)
        if (!isRetake && (c.status === 'completed' || c.status === 'inProgress' || exempted.has(nc))) continue
        if (have.has(nc)) continue
        const evalRes = evaluateCoursePrereqs(c.code, doneCodes, earnedCredits)
        const titlePair = pairedTitleByCode.get(nc)
        const pairNeedsTakeTogether =
          titlePair && !rawDone.has(normCode(titlePair.pairedCode)) ? [titlePair.pairedCode] : []
        const coreqs = [...new Set([...evalRes.coreqs, ...pairNeedsTakeTogether])]
        const labRole: 'lecture' | 'lab' | undefined = isStandaloneLabTitle(c.title)
          ? 'lab'
          : titlePair?.role
        const pairedCode = titlePair?.pairedCode ?? evalRes.coreqs[0]
        const cand: CompatibleCandidate = {
          code: c.code,
          title: c.title,
          units: c.units,
          plannedTerm: c.term,
          course: c,
          prereqStatus: evalRes.status,
          prereqText: evalRes.text,
          coreqs,
          unlocks: evalRes.unlocks,
          key: lookupCatalogKey(c.code),
          strm,
          labRole,
          pairedCode,
        }
        if (evalRes.status === 'unmet') locked.push(cand)
        else eligible.push(cand)
      }
    }
  }

  const rank = (x: CompatibleCandidate) => {
    const unlocksBonus = x.unlocks.length > 0 ? -Math.min(x.unlocks.length, 5) : 0
    const plannedRank = x.plannedTerm ? termOrder(x.plannedTerm) : 999999
    return [x.prereqStatus === 'met' ? 0 : 1, unlocksBonus, plannedRank] as const
  }
  eligible.sort((a, b) => {
    const ra = rank(a)
    const rb = rank(b)
    return ra[0] - rb[0] || ra[1] - rb[1] || ra[2] - rb[2] || a.code.localeCompare(b.code)
  })
  return { eligible, locked, doneCodes, earnedCredits, exempted }
}
