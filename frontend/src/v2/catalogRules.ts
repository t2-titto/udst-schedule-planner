import rawRules from './catalogRules.json'
import { normCode } from './prereq'

export type RuleNode =
  | string
  | { note: string }
  | { op: 'AND' | 'OR'; items: RuleNode[] }

export interface CohortRule {
  condition: string
  prerequisites: RuleNode | null
  corequisites: string[]
  notes: string[]
}

export interface CatalogRule {
  prerequisites?: RuleNode | null
  corequisites?: string[]
  antirequisites?: string[]
  cohort_rules?: CohortRule[]
  notes?: string[]
  unlocks?: string[]
}

const RULES = rawRules as Record<string, CatalogRule>

/** Instant lookup of the AI-parsed UDST Bachelor's catalog rules for `code`. */
export function getCatalogRule(code: string): CatalogRule | undefined {
  return RULES[normCode(code)]
}

/** Collect every course code referenced anywhere in a boolean rule tree. */
export function collectRuleCodes(node: RuleNode | null | undefined): string[] {
  if (!node) return []
  if (typeof node === 'string') return [normCode(node)]
  if ('note' in node) return []
  const out: string[] = []
  for (const item of node.items) {
    for (const c of collectRuleCodes(item)) {
      if (!out.includes(c)) out.push(c)
    }
  }
  return out
}

/**
 * Evaluate a nested AND/OR prerequisite tree against `done`.
 * Returns `true` when satisfied, `false` when certainly unmet, and `null` when a non-course note (like a placement score) could satisfy it.
 */
export function evalRuleTree(node: RuleNode | null | undefined, done: Set<string>): boolean | null {
  if (!node) return true
  if (typeof node === 'string') return done.has(normCode(node))
  if ('note' in node) return null
  const vals = node.items.map((it) => evalRuleTree(it, done))
  if (node.op === 'AND') {
    if (vals.some((v) => v === false)) return false
    if (vals.some((v) => v === null)) return null
    return true
  }
  if (vals.some((v) => v === true)) return true
  if (vals.some((v) => v === null)) return null
  return false
}

/** Render a boolean rule tree into concise readable text. */
export function formatRuleTree(node: RuleNode | null | undefined): string {
  if (!node) return ''
  if (typeof node === 'string') return normCode(node)
  if ('note' in node) return node.note
  const parts = node.items.map((it) => formatRuleTree(it)).filter(Boolean)
  if (parts.length <= 1) return parts[0] ?? ''
  const sep = node.op === 'OR' ? ' or ' : ' and '
  return parts.join(sep)
}
