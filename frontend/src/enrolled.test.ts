import { describe, expect, it } from 'vitest'
import { reconcile } from './enrolled'
import type { EnrolledRow } from './lock'

const row = (nbr: string, course: string, schedule: string[]): EnrolledRow => ({ row: 0, class_nbr: nbr, component: 'Laboratory', section: '1', course, schedule, status: 'Enrolled' })

describe('reconcile the drop page with the plan page', () => {
  const rows = [row('3613', 'INFS 2201 Database', ['Wednesday 4:00PM to 6:00PM']), row('3616', 'INFS 2201 Database', ['Thursday 8:00AM to 11:00AM']), row('3615', 'INFS 2201 Database', ['Sunday 8:00AM to 11:00AM']), row('9', 'ZZZ 1000 Other', ['Monday 9:00AM to 10:00AM'])]
  const summaries = { 'INFS 2201': 'Wednesday 4:00PM to 6:00PM Thursday 8:00AM to 11:00AM' }
  it('flags a class the plan page does not show', () => {
    const r = reconcile(rows, summaries)
    expect(r.unconfirmed.map((x) => x.class_nbr)).toEqual(['3615', '9'])
    expect(r.confirmed.map((x) => x.class_nbr)).toEqual(['3613', '3616'])
  })
  it('does not hold a course the plan page does not show at all (AETN 2302 case, 2026-11-23)', () => {
    const lab = [row('2216', 'AETN 2302 Applied Programming I', ['Tuesday 2:00PM to 4:00PM'])]
    expect(reconcile(lab, { 'INFS 2201': 'x' }).unconfirmed.map((x) => x.class_nbr)).toEqual(['2216'])
    expect(reconcile(lab, {}).confirmed).toEqual([])
  })
  it('trusts everything when the plan page was not read (undefined)', () => {
    expect(reconcile(rows, undefined).unconfirmed).toEqual([])
  })
  it('trusts a held course whose plan summary is empty', () => {
    expect(reconcile([row('5', 'QQQ 1000 X', ['Monday 9:00AM to 10:00AM'])], { 'QQQ 1000': '' }).confirmed.map((x) => x.class_nbr)).toEqual(['5'])
  })
})
