import { describe, expect, it } from 'vitest'
import { commit, redo, startHistory, undo } from './history'

describe('history', () => {
  it('undoes and redoes', () => {
    let h = startHistory(0)
    h = commit(h, 1, 0); h = commit(h, 2, 0)
    h = undo(h); expect(h.present).toBe(1)
    h = undo(h); expect(h.present).toBe(0)
    h = undo(h); expect(h.present).toBe(0)
    h = redo(h); expect(h.present).toBe(1)
  })
  it('a new change clears redo', () => {
    let h = commit(commit(startHistory(0), 1, 0), 2, 0)
    h = commit(undo(h), 9, 0)
    expect(h.future).toEqual([]); expect(redo(h).present).toBe(9)
  })
  it('keeps at most 50 steps', () => {
    let h = startHistory(0)
    for (let i = 1; i <= 80; i++) h = commit(h, i, 0)
    expect(h.past.length).toBe(50)
  })
  it('coalesces a tagged burst into one step', () => {
    let h = commit(startHistory(0), 1, 0)
    h = commit(h, 2, 100, 's'); h = commit(h, 3, 200, 's'); h = commit(h, 4, 300, 's')
    expect(undo(h).present).toBe(1)
    h = commit(h, 5, 5000, 's')
    expect(undo(h).present).toBe(4)
  })
  it('ignores an unchanged value', () => {
    const h = startHistory({ a: 1 })
    expect(commit(h, h.present, 0)).toBe(h)
  })
})
