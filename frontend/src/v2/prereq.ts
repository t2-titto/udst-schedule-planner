/**
 * PeopleSoft states prerequisites as free text, in two shapes seen so far:
 *   "MATH1020 OR AMPII Score of 85% is the pre-requisite of MATH1030, AETN2302, RSST3001."   (the expression, then what it is a prerequisite of)
 *   "Prerequisite(s): MATH1010 OR AMPII Score of 75%"                                          (the expression for this course)
 * The simple parts are parsed (course codes joined by OR or AND); anything else (a placement score) stays as a note. The raw text is always kept.
 */
export interface Prerequisite {
  raw: string
  /** Course codes in the expression ("MATH 1020"). */
  courses: string[]
  /** Other alternatives or conditions that are not a course ("AMPII Score of 85%"). */
  notes: string[]
  /** How the parts are joined: 'or' (any one), 'and' (all), or 'none' (a single part). */
  op: 'or' | 'and' | 'none'
}

const CODE = /^([A-Z]{2,6})\s?(\d{3,4}[A-Z]?)$/
export const normCode = (s: string): string => {
  const m = CODE.exec(s.trim())
  return m ? `${m[1]} ${m[2]}` : s.trim()
}

function parseExpression(expr: string): Omit<Prerequisite, 'raw'> {
  const op: Prerequisite['op'] = /\bOR\b/i.test(expr) ? 'or' : /\bAND\b/i.test(expr) ? 'and' : 'none'
  const parts = expr.split(/\s+(?:OR|AND)\s+/i).map((p) => p.trim()).filter(Boolean)
  const courses: string[] = []
  const notes: string[] = []
  for (const p of parts) (CODE.test(p) ? courses : notes).push(CODE.test(p) ? normCode(p) : p)
  return { courses, notes, op: parts.length > 1 ? op : 'none' }
}

/**
 * What this course needs, from the text of its "Course Requirements" page (or the Enrollment Requirement of its catalog page).
 * `code` is the course ("MATH 1030"); null when the text says nothing about it.
 */
export function prerequisiteOf(code: string, text: string): Prerequisite | null {
  const raw = text.replace(/\s+/g, ' ').trim()
  for (const part of raw.split(';').map((s) => s.trim()).filter(Boolean)) {
    const direct = /pre-?requisite\(?s?\)?:\s*(.+?)\.?$/i.exec(part)
    if (direct) return { raw, ...parseExpression(direct[1].replace(/\.$/, '')) }
    const stated = /^(.+?)\s+(?:is|are)\s+(?:a|the)\s+pre-?requisite(?:\s+(?:of|for)\s+(.+?))?\.?$/i.exec(part)
    if (stated) {
      if (!stated[2]) return { raw, ...parseExpression(stated[1]) }
      const targets = stated[2].split(/,|&|\band\b/i).map((t) => normCode(t.trim())).filter(Boolean)
      if (targets.includes(normCode(code))) return { raw, ...parseExpression(stated[1]) }
    }
  }
  return null
}

/**
 * Co-requisite course codes required alongside `code`, from PeopleSoft's requirement text.
 * Handles both `"AEMA2121 is the co-requisite for AEMA2221"` and `"Co-requisite(s): AEMA2121"` (including semicolon-separated clauses).
 */
export function corequisiteOf(code: string, text: string): string[] {
  const raw = text.replace(/\s+/g, ' ').trim()
  const out: string[] = []
  const add = (expr: string) => {
    for (const c of parseExpression(expr.replace(/\.$/, '')).courses) {
      if (!out.includes(c)) out.push(c)
    }
  }
  for (const part of raw.split(';').map((s) => s.trim()).filter(Boolean)) {
    const direct = /co-?requisite\(?s?\)?:\s*(.+?)\.?$/i.exec(part)
    if (direct) { add(direct[1]); continue }
    const stated = /^(.+?)\s+(?:is|are)\s+(?:a|the)\s+co-?requisite\s+(?:of|for)\s+(.+?)\.?$/i.exec(part)
    if (stated) {
      const targets = stated[2].split(/,|&|\band\b/i).map((t) => normCode(t.trim())).filter(Boolean)
      if (targets.includes(normCode(code))) add(stated[1])
    }
  }
  return out
}

/** True when the student has certainly met it, false when certainly not, null when a note (a placement score) leaves it open. */
export function isMet(req: Prerequisite, done: Set<string>): boolean | null {
  const have = req.courses.map((c) => done.has(c))
  if (req.op === 'and') return req.notes.length ? null : have.every(Boolean)
  if (have.some(Boolean)) return true
  return req.notes.length ? null : false
}
