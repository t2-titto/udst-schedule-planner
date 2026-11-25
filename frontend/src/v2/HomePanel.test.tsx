import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { Load } from './Load'

describe('the home screen', () => {
  it('has the two big choices and opens at once, before anything is read', () => {
    const html = renderToStaticMarkup(<Load onLoaded={() => {}} onSignOut={() => {}} onDegree={() => {}} />)
    expect(html).toContain("Build this term&#x27;s schedule")
    expect(html).toContain('Plan my degree')
    expect(html).toContain('What is known')
  })
})
