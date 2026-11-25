import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import wire from './fixtures/degree-wire.json'
import { buildDegree, type WireDegree } from './degree'
import { coursesOf, EMPTY_DRAFT, moveCourse, allowedTerms } from './draft'
import { commit, startHistory } from './history'
import { PlanBoard } from './PlanBoard'

const w = wire as unknown as WireDegree
const view = buildDegree(w, 'Winter 2027')
const text = (html: string) => html.replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ').trim()
const noop = () => {}
const render = (h = startHistory(EMPTY_DRAFT)) =>
  renderToStaticMarkup(<PlanBoard view={view} wire={w} openTerm="Winter 2027" prereqs={new Map()} history={h} onChange={noop} onUndo={noop} onRedo={noop} onDiscard={noop} onSave={noop} />)

describe('the term board', () => {
  it('opens on the open term year with the draft banner and three term blocks', () => {
    const t = text(render())
    expect(t).toContain('Draft')
    expect(t).toContain('Nothing is sent to PeopleSoft')
    for (const term of ['Fall 2026', 'Winter 2027', 'Spring 2027']) expect(t).toContain(term)
    expect(t).toContain('2026–27')
  })
  it('shows a moved course in its new term with the change count', () => {
    const soft = coursesOf(view).find((c) => c.code === 'SOFT 2301')!
    const opts = w.rows.find((r) => r.row === soft.row)!.term_choice!.options.map((o) => ({ term: o.label, code: o.code }))
    const d = moveCourse(EMPTY_DRAFT, soft, 'Winter 2027', allowedTerms(soft, opts, 'Winter 2027'))
    const html = render(commit(startHistory(EMPTY_DRAFT), d, 1))
    expect(text(html)).toContain('1 course moved')
    expect(html).toContain('moved in draft')
  })
  it('Save stays off, with the reason, until the schedule is loaded', () => {
    const html = renderToStaticMarkup(<PlanBoard canSave={false} view={view} wire={w} openTerm="Winter 2027" prereqs={new Map()} history={startHistory(EMPTY_DRAFT)} onChange={noop} onUndo={noop} onRedo={noop} onDiscard={noop} onSave={noop} />)
    expect(text(html)).toContain('To save, open this term')
  })
  it('only planned courses are draggable', () => {
    const html = render()
    expect((html.match(/draggable="true"/g) ?? []).length).toBeGreaterThan(0)
    expect(html).toContain('draggable="false"')
  })
})
