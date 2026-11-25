import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { AddCourse } from './AddCourse'

describe('the add-a-course screen', () => {
  it('opens at once, says only degree courses are listed and offers a search box', () => {
    const html = renderToStaticMarkup(<AddCourse term="Winter 2027" inSchedule={[]} extras={[]} onAdded={() => {}} onKeep={() => {}} onBack={() => {}} />)
    expect(html).toContain('Add a course to Winter 2027')
    expect(html).toContain('Only courses of your own degree')
    expect(html).toContain('aria-label="Search your degree courses"')
  })
})
