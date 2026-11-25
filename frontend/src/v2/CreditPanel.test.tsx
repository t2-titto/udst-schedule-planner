import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { CreditPanel } from './CreditPanel'
import { creditsFor } from './credits'
import { emptyState, type PCourse } from './model'

const held = { id: 'h', optionNo: 'Enrolled', status: 'Open', components: [] }
const enrolled = (code: string, units: number): PCourse => ({ code, title: '', term: 'Winter 2027', options: [], held, units })
const real = [enrolled('DACS 2202', 4), enrolled('DSAI 2201', 3), enrolled('INFS 2201', 3), enrolled('INFS 3102', 3), enrolled('GARC 2001', 3)]
const text = (html: string) => html.replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ').trim()

describe('what the student reads when over the limit', () => {
  const over = creditsFor([...real, { code: 'SOFT 2301', title: '', term: 'Winter 2027', options: [], units: 3 }], emptyState(), 'Winter 2027')
  const html = renderToStaticMarkup(<CreditPanel credits={over} onRemoveEnrolled={() => undefined} />)
  const t = text(html)
  it('says how many credits it is over and that Apply is blocked', () => {
    expect(t).toContain('19 credits is 1 over the 18 PeopleSoft allows')
    expect(t).toContain('Apply is blocked')
  })
  it('lists the planned course first and says it is enough on its own', () => {
    expect(t.indexOf('SOFT 2301')).toBeLessThan(t.indexOf('DACS 2202 ('))
    expect(t).toContain('enough on its own')
  })
  it('warns that taking out an enrolled course drops it', () => {
    expect(t).toContain('drops you from the course')
  })
  it('says PeopleSoft counts a course the student unticked', () => {
    expect(t).toContain('even one you unticked here')
  })
})
