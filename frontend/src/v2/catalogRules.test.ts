import { describe, expect, it } from 'vitest'
import { collectRuleCodes, evalRuleTree, formatRuleTree, getCatalogRule } from './catalogRules'

describe('catalogRules', () => {
  it('loads parsed rules for UDST courses including prerequisites, corequisites, cohort rules, and unlocks', () => {
    const aema2221 = getCatalogRule('AEMA2221')!
    expect(aema2221.corequisites).toEqual(['AEMA 2121'])
    expect(collectRuleCodes(aema2221.prerequisites)).toEqual(['AEMA 1102'])

    const math1030 = getCatalogRule('MATH 1030')!
    expect(evalRuleTree(math1030.prerequisites, new Set(['MATH 1020']))).toBe(true)
    expect(evalRuleTree(math1030.prerequisites, new Set())).toBeNull() // AMPII Score alternative
    expect(math1030.unlocks).toContain('MATH 2010')

    const aema2133 = getCatalogRule('AEMA 2133')!
    expect(aema2133.cohort_rules?.length).toBe(2)
  })

  it('evaluates nested AND/OR trees and formats them', () => {
    const tree = { op: 'AND' as const, items: ['ACCT 1001', { op: 'OR' as const, items: ['MATH 1010', 'MATH 1020'] }] }
    expect(evalRuleTree(tree, new Set(['ACCT 1001']))).toBe(false)
    expect(evalRuleTree(tree, new Set(['ACCT 1001', 'MATH 1020']))).toBe(true)
    expect(formatRuleTree(tree)).toBe('ACCT 1001 and (MATH 1010 or MATH 1020)')
  })
})
