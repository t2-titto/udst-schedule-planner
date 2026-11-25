import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import wire from './fixtures/degree-wire.json'
import { buildDegree, type WireDegree } from './degree'
import { allowedTerms, coursesOf, EMPTY_DRAFT, moveCourse } from './draft'
import { PlanSave } from './PlanSave'

const w = wire as unknown as WireDegree
const view = buildDegree(w, 'Winter 2027')
const text = (html: string) => html.replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ').trim()
const noop = () => {}
const soft = coursesOf(view).find((c) => c.code === 'SOFT 2301')!
const opts = w.rows.find((r) => r.row === soft.row)!.term_choice!.options.map((o) => ({ term: o.label, code: o.code }))
const draft = moveCourse(EMPTY_DRAFT, soft, 'Fall 2027', allowedTerms(soft, opts, 'Winter 2027'))
const render = (d = draft) => renderToStaticMarkup(<PlanSave view={view} wire={w} openTerm="Winter 2027" draft={d} onDone={noop} onCancel={noop} />)

describe('the save review', () => {
  it('lists the move, says it is planning only and offers the save and keep-as-draft buttons', () => {
    const t = text(render())
    expect(t).toContain('SOFT 2301 Spring 2027 → Fall 2027')
    expect(t).toContain('Fall 2027 will have 15 credits')
    expect(t).toContain('does not enrol you')
    expect(t).toContain('Save 1 change to PeopleSoft')
    expect(t).toContain('Keep as draft')
    expect(render()).not.toContain('disabled=""')
  })
  it('an empty draft has nothing to save and the button is off', () => {
    const html = render(EMPTY_DRAFT)
    expect(html).toContain('disabled=""')
  })
})
