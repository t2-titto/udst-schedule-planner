import { describe, expect, it } from 'vitest'
import { lanesOf } from './Calendar'
import type { PBlock } from './model'

const b = (code: string, day: number, start: number, end: number): PBlock => ({ courseCode: code, classNbr: code, kind: 'Lecture', room: '', instructors: [], day, start, end, locked: false, held: false })

describe('overlapping cards', () => {
  it('puts the longer card behind and the shorter in front', () => {
    const big = b('A', 1, 540, 660), small = b('B', 1, 600, 630)
    const l = lanesOf([small, big])
    expect(l.get('A-A-1-540')).toMatchObject({ lane: 0, lanes: 2 })
    expect(l.get('B-B-1-600')).toMatchObject({ lane: 1, lanes: 2 })
  })
  it('leaves cards that do not touch alone, and other days apart', () => {
    const l = lanesOf([b('A', 1, 540, 600), b('B', 1, 600, 660), b('C', 2, 540, 600)])
    expect([...l.values()].every((x) => x.lanes === 1)).toBe(true)
  })
  it('chains three cards into one cluster', () => {
    const l = lanesOf([b('A', 1, 540, 600), b('B', 1, 590, 650), b('C', 1, 640, 700)])
    expect(new Set([...l.values()].map((x) => x.cluster)).size).toBe(1)
  })
})
