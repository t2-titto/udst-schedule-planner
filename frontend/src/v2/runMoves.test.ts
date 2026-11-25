import { beforeEach, describe, expect, it, vi } from 'vitest'

const calls: string[] = []
const status = { armed: false, confirmMode: 'click', consent: { version: 'v', statements: [{ id: 'a' }, { id: 'b' }] } }
let failPreviewAt = -1
let previews = 0

vi.mock('../api', () => ({
  ApiError: class extends Error { status: number; constructor(m: string, s: number) { super(m); this.status = s } },
  api: {
    writeStatus: async () => status,
    writeArm: async (acks: string[]) => { calls.push(`arm ${acks.join(',')}`) },
    writeDisarm: async () => { calls.push('disarm') },
    writePreview: async (tool: string, args: Record<string, unknown>, _e: unknown, _c: unknown, credits: { term: string }) => {
      const i = previews++
      calls.push(`preview ${tool} ${args.term_code} ${credits.term}`)
      if (i === failPreviewAt) throw new Error('over the limit')
      return { token: `t${i}` }
    },
    writeConfirm: async (t: string) => { calls.push(`confirm ${t}`) },
  },
}))

import { executeMoves } from './runMoves'
import type { Move } from './savePlan'

const mv = (code: string, to: string, toCode: string): Move => ({
  course: { row: 1, year: 1, code, title: '', category: '', units: 3, term: 'Spring 2027', status: 'planned', plannable: true, hasRequirement: false },
  from: 'Spring 2027', to, toCode, fromCode: '1275', year: 1, index: 8, after: 6, unknown: [],
})

beforeEach(() => { calls.length = 0; previews = 0; failPreviewAt = -1; status.armed = false; status.confirmMode = 'click' })

describe('sending plan moves', () => {
  it('arms, previews and confirms each move in order, and always disarms', async () => {
    const steps: string[] = []
    const r = await executeMoves([mv('A 1', 'Fall 2027', '1278'), mv('B 2', 'Winter 2028', '1281')], (i, s) => steps.push(`${i}:${s}`))
    expect(r.error).toBeNull()
    expect(r.done).toHaveLength(2)
    expect(calls).toEqual(['arm a,b', 'preview preview_plan_term 1278 Fall 2027', 'confirm t0', 'preview preview_plan_term 1281 Winter 2028', 'confirm t1', 'disarm'])
    expect(steps).toEqual(['0:working', '0:saved', '1:working', '1:saved'])
  })
  it('stops at the first failure, does not send the rest, and still disarms', async () => {
    failPreviewAt = 0
    const steps: string[] = []
    const r = await executeMoves([mv('A 1', 'Fall 2027', '1278'), mv('B 2', 'Winter 2028', '1281')], (i, s) => steps.push(`${i}:${s}`))
    expect(r.error).toContain('A 1')
    expect(r.error).toContain('over the limit')
    expect(r.done).toEqual([])
    expect(calls.filter((c) => c.startsWith('confirm'))).toEqual([])
    expect(calls[calls.length - 1]).toBe('disarm')
    expect(steps).toEqual(['0:working', '0:failed', '1:not sent'])
  })
  it('a server that wants a typed phrase is refused before anything is sent', async () => {
    status.confirmMode = 'phrase'
    const r = await executeMoves([mv('A 1', 'Fall 2027', '1278')], () => {})
    expect(r.error).toContain('typed phrase')
    expect(calls.filter((c) => c.startsWith('preview'))).toEqual([])
  })
  it('does not arm twice when writes are already on', async () => {
    status.armed = true
    await executeMoves([mv('A 1', 'Fall 2027', '1278')], () => {})
    expect(calls.filter((c) => c.startsWith('arm'))).toEqual([])
  })
})
