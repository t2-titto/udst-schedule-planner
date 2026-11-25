/**
 * The page and the server must agree on every request body. These tests build the requests with the page's own code and compare them with
 * committed samples (fixtures/contract-*.json); the server's tests (`contract_*` in write.rs) read the very same files and must parse and accept
 * them. Change a request's shape and one side fails until the sample is updated, which then fails the other side if it no longer parses.
 * Regenerate with:  WRITE_CONTRACT=1 npx vitest run src/v2/contract.test.ts
 */
import { describe, expect, it } from 'vitest'
import type { ClassOption, Component, Course } from '../engine/types'
import { buildPlan } from '../plan'
import { planSwapRounds } from '../swapRounds'
import type { EnrolledState } from '../enrolled'
import { buildApplyRequest, wireProblems } from './applyRequest'
import { creditsFor } from './credits'
import { emptyState, type AddPlan } from './model'
import { moveRequest, type Move } from './savePlan'
import applySample from './fixtures/contract-apply-plan.json'
import previewSample from './fixtures/contract-preview-plan.json'
import catalogSample from './fixtures/contract-catalog-read.json'

const maybeWrite = async (name: string, obj: unknown) => {
  const env = (globalThis as unknown as { process?: { env?: Record<string, string> } }).process?.env
  if (!env?.WRITE_CONTRACT) return
  const fs = (await import(/* @vite-ignore */ ['node', 'fs'].join(':'))) as unknown as { writeFileSync: (p: URL, s: string) => void }
  fs.writeFileSync(new URL(`./fixtures/${name}`, import.meta.url), JSON.stringify(obj, null, 2) + '\n')
}

const comp = (nbr: string, kind: string, day: number, start: number, end: number): Component => ({ dates: '', kind, classNbr: nbr, section: nbr, room: 'R', instructors: ['A'], meetings: [{ day, start, end }], seatsOpen: 5, seatsTotal: 20 })
const opt = (id: string, ...c: Component[]): ClassOption => ({ id, optionNo: id, status: 'Open', optionRow: undefined, components: c })
const course = (code: string, options: ClassOption[]): Course => ({ code, title: '', term: 'Winter 2099', key: { strm: '9999', crse_id: '000002', crse_offer_nbr: '1', acad_career: 'CRED', institution: 'TESTI' }, row: undefined, options }) as Course
const nothingEnrolled: EnrolledState = { term: 'Winter 2099', termRow: -1, rows: [], options: {}, unconfirmed: [] }

function applyPlanRequest() {
  const o = opt('n', comp('9', 'Lecture', 1, 540, 600), comp('10', 'Laboratory', 2, 540, 600))
  const c = course('TEST 1001', [o])
  const sched = { picks: [{ course: c, option: o, courseIndex: 0, instructorScore: 0, limiting: [] }], blocks: [], freeDays: [], scores: { instructor: 0, timing: 0, freeDay: 0, total: 0 } }
  const plan = buildPlan(sched, {}, [], { selected: [], hidden: [], courses: [], live: new Set(['TEST 1001']) })
  const rp = planSwapRounds([], [], [])
  const add: AddPlan = { year: 1, index: 8, toCode: '1271', to: 'Winter 2099' }
  return buildApplyRequest(plan, rp, nothingEnrolled, plan.drops, creditsFor([{ code: 'TEST 1001', title: '', term: 'Winter 2099', options: [o], units: 3, addPlan: add }], emptyState(), 'Winter 2099'), { 'TEST 1001': add })
}

function previewPlanRequest() {
  const m: Move = {
    course: { row: 19, year: 1, code: 'TEST 1001', title: '', category: '', units: 3, term: 'Spring 2099', status: 'planned', plannable: true, hasRequirement: false },
    from: 'Spring 2099', to: 'Fall 2099', toCode: '1278', fromCode: '1275', year: 1, index: 8, after: 6, unknown: [],
  }
  const { args, credits, expect } = moveRequest(m)
  return { tool: 'preview_plan_term', args, credits, expect }
}

describe('the requests the page sends are the samples the server is tested against', () => {
  it('Apply with a plan step', async () => {
    const req = JSON.parse(JSON.stringify(applyPlanRequest()))
    await maybeWrite('contract-apply-plan.json', req)
    expect(wireProblems(req)).toEqual([])
    expect(req).toEqual(applySample)
  })
  it('the plan preview', async () => {
    const req = JSON.parse(JSON.stringify(previewPlanRequest()))
    await maybeWrite('contract-preview-plan.json', req)
    expect(req).toEqual(previewSample)
  })
  it('a catalog read', async () => {
    const req = { tool: 'catalog_search', args: { keyword: 'TEST 1001' } }
    await maybeWrite('contract-catalog-read.json', req)
    expect(req).toEqual(catalogSample)
  })
  it('a deliberate wrong shape is caught on this side too', () => {
    const wrong = JSON.parse(JSON.stringify(applySample)) as { plans: { termCode: unknown }[]; enrols: { pick: { row: number } }[] }
    wrong.enrols[0].pick.row = -1
    expect(wireProblems(wrong).length).toBeGreaterThan(0)
    expect(JSON.parse(JSON.stringify({ ...applySample, plans: [{ ...applySample.plans[0], termCode: 1271 }] }))).not.toEqual(applySample)
  })
})

import waitlistSample from './fixtures/contract-waitlist.json'
import { normalizeCourses as normWait } from '../engine/normalize'
import { isJoinable as joinable } from '../engine/solver'

describe('a real waitlist class as PeopleSoft\'s catalog shows it (read-only capture, 2026-11-23)', () => {
  it('is read as a joinable waitlist option, not as an open class', () => {
    const c = normWait({ courses: [{ row: 1, key: { strm: '1271' }, sections: waitlistSample }] }).courses[0]
    const o = c.options[0]
    expect(o.status).toBe('Wait List')
    expect(o.waitlist).toBe(true)
    expect(o.components.map((x) => x.waitPlaces)).toEqual([2, 2])
    expect(o.components.every((x) => x.seatsOpen === null)).toBe(true)
    expect(joinable(o)).toBe(true)
  })
})
