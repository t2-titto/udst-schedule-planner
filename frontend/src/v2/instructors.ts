import { fmtCompact } from '../engine/normalize'
import { DAY_SHORT, type ClassOption, type Component } from '../engine/types'

/** A class the way the instructor list shows it. */
export interface ClassInfo {
  nbr: string
  isLab: boolean
  kind: string
  section: string
  when: string
  room: string
  instructors: string[]
  /** The classes that are picked together with this one: a lab's lectures, a lecture's labs. */
  links: { nbr: string; kind: string; when: string; instructors: string[] }[]
}

export interface InstructorStat {
  name: string
  classes: ClassInfo[]
  lectures: number
  labs: number
}

export const isLabKind = (kind: string): boolean => /lab/i.test(kind)

export const whenOf = (c: Component): string =>
  [...new Set(c.meetings.map((m) => `${DAY_SHORT[m.day]} ${fmtCompact(m.start, m.end)}`))].join(', ')

/**
 * Every instructor of a course with the classes they teach (each class once, however many combinations it appears in), how many are
 * lectures and how many labs, and for each class what it is picked together with.
 */
export function instructorStats(options: ClassOption[]): InstructorStat[] {
  const info = new Map<string, ClassInfo>()
  for (const o of options) {
    for (const c of o.components) {
      let i = info.get(c.classNbr)
      if (!i) {
        i = { nbr: c.classNbr, isLab: isLabKind(c.kind), kind: c.kind, section: c.section, when: whenOf(c), room: c.room, instructors: c.instructors, links: [] }
        info.set(c.classNbr, i)
      }
      for (const other of o.components) {
        if (other.classNbr === c.classNbr || isLabKind(other.kind) === isLabKind(c.kind)) continue
        if (!i.links.some((l) => l.nbr === other.classNbr)) i.links.push({ nbr: other.classNbr, kind: other.kind, when: whenOf(other), instructors: other.instructors })
      }
    }
  }
  const byName = new Map<string, InstructorStat>()
  for (const i of info.values()) {
    for (const name of i.instructors) {
      const st = byName.get(name) ?? { name, classes: [], lectures: 0, labs: 0 }
      st.classes.push(i)
      if (i.isLab) st.labs++
      else st.lectures++
      byName.set(name, st)
    }
  }
  return [...byName.values()]
}
