import { describe, expect, it } from 'vitest'
import { PICKS_KEY, readPicks, writePicks, type SavedPicks } from './savedPicks'

function memory(): Storage {
  const m = new Map<string, string>()
  return {
    getItem: (k: string) => (m.has(k) ? m.get(k)! : null),
    setItem: (k: string, v: string) => { m.set(k, v) },
    removeItem: (k: string) => { m.delete(k) },
    clear: () => m.clear(),
    key: () => null,
    get length() { return m.size },
  } as Storage
}

const picks: SavedPicks = {
  term: 'Winter 2027',
  state: { removed: ['AETN 2302'], unlocked: [], choice: { 'AEMA 2311': 'opt-2' }, pins: [] },
  added: [],
  excluded: { 'AEMA 3121': ['opt-9'] },
}

describe('the schedule kept for a refresh', () => {
  it('comes back as it was written, for the same term', () => {
    const s = memory()
    writePicks(picks, s)
    expect(readPicks(s)).toEqual(picks)
  })
  it('is absent when nothing was written', () => {
    expect(readPicks(memory())).toBeNull()
  })
  it('is ignored when the stored shape is not one this version writes', () => {
    const s = memory()
    s.setItem(PICKS_KEY, JSON.stringify({ term: 'Winter 2027', state: { removed: 'x' }, added: [], excluded: {} }))
    expect(readPicks(s)).toBeNull()
    s.setItem(PICKS_KEY, '{not json')
    expect(readPicks(s)).toBeNull()
  })
  it('a blocked store does not throw', () => {
    const blocked = { getItem: () => { throw new Error('blocked') }, setItem: () => { throw new Error('blocked') } }
    expect(() => writePicks(picks, blocked)).not.toThrow()
    expect(readPicks(blocked)).toBeNull()
  })
})
