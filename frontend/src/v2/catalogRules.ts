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
 * Evaluate a nested AND/OR prerequisite tree against `done` (and `earnedCredits` for credit-minimum notes).
 * Returns `true` when satisfied, `false` when certainly unmet, and `null` when a placement score/entry rule could satisfy it.
 */
export function evalRuleTree(node: RuleNode | null | undefined, done: Set<string>, earnedCredits?: number | null): boolean | null {
  if (!node) return true
  if (typeof node === 'string') return done.has(normCode(node))
  if ('note' in node) {
    const m = node.note.trim().match(/^minimum\s+(\d+)\s*credits$/i)
    if (m && typeof earnedCredits === 'number' && Number.isFinite(earnedCredits)) {
      return earnedCredits >= Number(m[1])
    }
    return null
  }
  const vals = node.items.map((it) => evalRuleTree(it, done, earnedCredits))
  if (node.op === 'AND') {
    if (vals.some((v) => v === false)) return false
    if (vals.some((v) => v === null)) return null
    return true
  }
  if (vals.some((v) => v === true)) return true
  if (vals.some((v) => v === null)) return null
  return false
}

/** Render a boolean rule tree into concise readable text with explicit grouping for mixed operators. */
export function formatRuleTree(node: RuleNode | null | undefined, parentOp?: 'AND' | 'OR'): string {
  if (!node) return ''
  if (typeof node === 'string') return normCode(node)
  if ('note' in node) return node.note
  const parts = node.items.map((it) => formatRuleTree(it, node.op)).filter(Boolean)
  if (parts.length <= 1) return parts[0] ?? ''
  const sep = node.op === 'OR' ? ' or ' : ' and '
  const joined = parts.join(sep)
  return parentOp && parentOp !== node.op ? `(${joined})` : joined
}

/** True when a prerequisite tree includes an external placement/program-entry note rather than only course/credit checks. */
export function isPlacementRule(node: RuleNode | null | undefined): boolean {
  if (!node || typeof node === 'string') return false
  if ('note' in node) return !/^minimum\s+\d+\s*credits$/i.test(node.note.trim())
  return node.items?.some((it) => isPlacementRule(it)) ?? false
}
