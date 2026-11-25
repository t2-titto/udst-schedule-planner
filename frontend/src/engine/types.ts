export const DAY_NAMES = [
  'Sunday',
  'Monday',
  'Tuesday',
  'Wednesday',
  'Thursday',
  'Friday',
  'Saturday',
] as const
export const DAY_SHORT = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'] as const

/** The university week the planner scores free days on. Friday and Saturday join only if a class meets then. */
export const BASE_DAYS = [0, 1, 2, 3, 4]

export interface Meeting {
  day: number // 0 = Sunday
  start: number // minutes after midnight
  end: number
}

export interface Component {
  /** The date range PeopleSoft shows, "05/01/2027 - 14/04/2027" (day first). Often only on the lecture. */
  dates: string
  kind: string // Lecture, Laboratory, ...
  classNbr: string
  section: string
  room: string
  instructors: string[] // every instructor of this component; never empty ('Staff' when unknown)
  meetings: Meeting[]
  seatsOpen: number | null
  seatsTotal: number | null
  /** Places left on the waitlist ("Waitlist Available Places 3 of 10"); null when the class shows none. */
  waitPlaces?: number | null
}

/** One enrollable choice for a course: a lecture, usually with a lab, picked together. */
export interface ClassOption {
  id: string
  /** Row of this option on PeopleSoft's course page. Present only for data fetched live; needed to enrol. */
  optionRow?: number
  optionNo: string
  status: string
  /** PeopleSoft says "Wait List" for this option: joining it puts the student on a waitlist, not in the class. */
  waitlist?: boolean
  /** The option is what the student holds on a waitlist (held, not enrolled). */
  waitlisted?: boolean
  components: Component[]
}

/** PeopleSoft's identifier for one course in one term. A catalogue code, not personal; lets a refetch skip the slow way. */
export interface CourseKey {
  strm: string
  crse_id: string
  crse_offer_nbr: string
  acad_career: string
  institution: string
}

export interface Course {
  units?: string
  grading?: string
  description?: string
  key?: CourseKey
  /** Row of this course in the Plan & Enrol class list. Present only for data fetched live; needed to enrol. */
  row?: number
  code: string
  title: string
  term: string
  options: ClassOption[]
}

export interface TimingSettings {
  /** Start and end of the ideal window, in minutes after midnight. Score is 10 inside it. */
  start: number
  end: number
  /** Minutes over which the score falls linearly from 10 to 5 outside the window. 0 = hard edge. */
  fade: number
}

export interface Weights {
  instructor: number
  timing: number
  freeDay: number
  /** How much a change (a swap or a new enrolment) costs a schedule in the ranking. 0 = ignore changes. */
  changes?: number
}

export interface Rules {
  excludeFull: boolean
  /** Minimum minutes between two classes on the same day. */
  minGap: number
  /** No class may end after this (minutes), or null for no limit. */
  latestEnd: number | null
  /** Courses the latest-end rule does not apply to (course codes). */
  latestEndExempt: string[]
  minFreeDays: number
  /**
   * Classes in an "Online…" room stay in the schedule and still may not overlap anything, but are left out of the
   * timing score, the instructor score, the free-day count, the minimum gap and the latest-end rule.
   */
  ignoreOnline: boolean
  /** Waitlist support, off by default (UNVERIFIED against a real waitlist enrolment). */
  joinWaitlist?: boolean
}

export interface Settings {
  /** Keyed `${courseCode}::${instructorName}`. Missing = 5. */
  instructorScores: Record<string, number>
  timing: TimingSettings
  /** Index 0..6 by day. Value for a day being completely free of classes. */
  freeDayScores: number[]
  weights: Weights
  rules: Rules
  topN: number
}

export const DEFAULT_SETTINGS: Settings = {
  instructorScores: {},
  timing: { start: 8 * 60, end: 12 * 60, fade: 3 * 60 },
  freeDayScores: [5, 5, 5, 5, 5, 5, 5],
  weights: { instructor: 5, timing: 5, freeDay: 5, changes: 0 },
  rules: { excludeFull: true, minGap: 0, latestEnd: null, latestEndExempt: [], minFreeDays: 1, ignoreOnline: false },
  topN: 10,
}

export const NEUTRAL = 5
export const instructorKey = (courseCode: string, name: string) => `${courseCode}::${name}`
