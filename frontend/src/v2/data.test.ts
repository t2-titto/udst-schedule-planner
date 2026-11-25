import { describe, expect, it } from 'vitest'
import { keepJoinable, openTerm, termOrder } from './data'
import type { ClassOption } from '../engine/types'

describe('open term', () => {
  it('is the term with classes available', () => {
    expect(openTerm([{ term: 'Winter 2027', status: 'Classes available' }, { term: 'Winter 2027', status: 'Classes available' }, { term: 'Fall 2027', status: 'Classes available' }, { term: 'Fall 2026', status: 'Enrolled' }])).toBe('Winter 2027')
  })
  it('falls back to the latest term you are enrolled in when nothing is left to add', () => {
    expect(openTerm([{ term: 'Fall 2026', status: 'Enrolled' }, { term: 'Winter 2027', status: 'Enrolled' }, { term: 'Fall 2028', status: 'No classes available' }])).toBe('Winter 2027')
  })
  it('orders terms by academic year', () => {
    expect(termOrder('Fall 2026') < termOrder('Winter 2027')).toBe(true)
    expect(termOrder('Spring 2027') < termOrder('Fall 2027')).toBe(true)
  })
})

describe('keepJoinable (unverified waitlists)', () => {
  const c = (nbr: string, places: number | null) => ({ dates: '', kind: 'Lecture', classNbr: nbr, section: '1', room: '', instructors: ['A'], meetings: [], seatsOpen: null, seatsTotal: null, waitPlaces: places })
  const open: ClassOption = { id: 'o', optionNo: '1', status: 'Open', components: [c('1', null)] }
  const wl: ClassOption = { id: 'w', optionNo: '2', status: 'Wait List', waitlist: true, components: [c('2', 3)] }
  const none: ClassOption = { id: 'n', optionNo: '3', status: 'Wait List', waitlist: true, components: [c('3', 0)] }
  it('with the switch off a Wait List option never reaches a list', () => {
    expect(keepJoinable([open, wl, none], false).map((o) => o.id)).toEqual(['o'])
  })
  it('with it on, only a waitlist with a place stays', () => {
    expect(keepJoinable([open, wl, none], true).map((o) => o.id)).toEqual(['o', 'w'])
  })
})
