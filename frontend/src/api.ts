import type { ApplyRequest } from './v2/applyRequest'
export class ApiError extends Error {
  status: number
  constructor(message: string, status: number) {
    super(message)
    this.status = status
  }
}

export interface Status {
  binaryFound: boolean
  signedIn: boolean
  userHint: string | null
  demo?: boolean
}

export interface WriteStatus {
  demo: boolean
  armed: boolean
  idleSecondsLeft: number | null
  /** The operation the server is running right now, if any. Asking for the status never waits for it. */
  busy: { op: string; seconds: number } | null
  queued: number
  confirmMode: 'click' | 'phrase'
  consent: { version: string; statements: { id: string; text: string }[] }
}

/** The credit check a plan write carries: the destination term's total after the move, as the page counted it (the server refuses over the limit). */
export interface PlanCreditsReq {
  term: string
  limit: number
  after: number
  unknown: string[]
}

export interface WritePlan {
  kind: string
  term: string
  class_numbers: string[]
  summary: string
  steps: { component: string; icaction: string }[]
  warnings: string[]
  required_phrase: string
}

export interface SeatReading {
  classNbr: string
  kind: string
  open: number | null
  total: number | null
  seatsText: string
  optionStatus: string
  seenOpen: number | null
  dropped: boolean
  ok: boolean
}

/** Seats read fresh from PeopleSoft just before a write step. */
export interface SeatReport {
  course: string
  term: string
  readAtMs: number
  readings: SeatReading[]
}

export interface Preview {
  plan: WritePlan
  token: string
  ttl_secs: number
  seats?: SeatReport[]
}

export interface Outcome {
  kind: string
  classNumbers: string[]
  messages: string[]
  detail: unknown
  /** True when the write may or may not have gone through (the answer never arrived). `verify` then says what PeopleSoft shows. */
  unknown?: boolean
  error?: string
  said?: string[]
  /**
   * What PeopleSoft's own pages say afterwards, the only real proof. After an enrol: `rows`, one per chosen course. After a
   * drop: `state` ('absent' = no longer on the term's enrolled list, 'still_present', or 'unknown' = could not be read).
   */
  verify?: {
    readAtMs?: number
    rows?: { course: string; status: string; enrolled: boolean }[]
    error?: string
    state?: 'absent' | 'still_present' | 'unknown'
    classNbr?: string
    /** The term no longer appears on PeopleSoft's drop page: that was its last class. */
    termLeft?: boolean
    remaining?: string[]
  }
}



export interface PlanProgress {
  id: number
  phase: string
  finished: boolean
  error: string | null
  items: { id: string; kind: 'drop' | 'swap' | 'enrol'; course: string; status: 'pending' | 'working' | 'done' | 'error'; note: string; round?: number }[]
}




/** What the person chose for one course and the seats they saw, so the server can read the seats fresh and compare. */
export interface SeatCheckReq {
  course: string
  key: unknown
  classNbrs: string[]
  seen: Record<string, number | null>
}


export interface FetchStats {
  direct: number
  opened: number
  /** Courses opened on extra read-only sessions at the same time (a cold fetch only). */
  parallel?: number
  helpers?: number
  listVisited: boolean
  ms: number
}

export interface ClassRow {
  row: number
  course_label: string
  term: string
  status: string
  schedule_summary: string
}

export interface ClassList {
  term_label: string
  rows: ClassRow[]
  can_submit: boolean
}

const SESSION_KEY = 'schedule-planner:v2:session'
let memSessionId: string | null = null

export function newSessionId(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') return crypto.randomUUID()
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 12)}`
}

/** Per-tab / per-window session id (lives in sessionStorage so private windows and other browsers start fresh). */
export function getSessionId(): string {
  if (memSessionId) return memSessionId
  try {
    const existing = window.sessionStorage.getItem(SESSION_KEY)
    if (existing) { memSessionId = existing; return existing }
    const fresh = newSessionId()
    window.sessionStorage.setItem(SESSION_KEY, fresh)
    memSessionId = fresh
    return fresh
  } catch {
    memSessionId = newSessionId()
    return memSessionId
  }
}

export function setSessionId(id: string): void {
  memSessionId = id
  try { window.sessionStorage.setItem(SESSION_KEY, id) } catch { /* ignore */ }
}

async function call<T>(path: string, init?: RequestInit): Promise<T> {
  let res: Response
  try {
    res = await fetch(`/api${path}`, {
      ...init,
      cache: 'no-store',
      credentials: 'same-origin',
      headers: { 'content-type': 'application/json', 'x-planner-request': '1', 'x-planner-session': getSessionId() },
    })
  } catch {
    throw new ApiError('The planner server is not reachable. Is it running?', 0)
  }
  const raw = await res.text().catch(() => '')
  let body: unknown = null
  try {
    body = raw ? JSON.parse(raw) : null
  } catch {
    body = null
  }
  if (!res.ok) {
    // The server's own message when it sent one; for a body it could not read (a 422 names the field) the first line of what it said.
    const plain = raw.trim().split('\n')[0].slice(0, 160)
    const msg = body && typeof body === 'object' && 'error' in body ? String((body as { error: unknown }).error) : plain ? `Request failed (${res.status}): ${plain}` : `Request failed (${res.status})`
    throw new ApiError(msg, res.status)
  }
  if (raw.trim() && body === null) throw new ApiError('The server answered with something that is not JSON. Is the planner up to date? Reload the page.', res.status)
  return body as T
}

export const api = {
  status: () => call<Status>('/status'),
  // The password goes to this machine's own server only; it is never kept in page state or storage.
  login: (username: string, password: string) =>
    call<{ signedIn: boolean; userHint: string }>('/session', { method: 'POST', body: JSON.stringify({ username, password }) }),
  logout: () => call<{ signedIn: boolean }>('/session', { method: 'DELETE' }),
  classes: () => call<ClassList>('/classes'),
  /** The class list and, when `term` is given, that term's enrolled list, read on two sessions at once. */
  refresh: (term?: string) =>
    call<{ classes: ClassList; enrolled: { termRow: number; classes: unknown[] } | null; stats: { via: string; ms: number; listMs: number } }>('/refresh', { method: 'POST', body: JSON.stringify({ term }) }),
  writeStatus: (signal?: AbortSignal) => call<WriteStatus>('/write/status', { signal }),
  /** Drops, then all swaps through one commit, then enrolments: one job, followed with `planProgress`. */
  planApply: (body: ApplyRequest) => call<{ started: boolean }>('/write/plan/apply', { method: 'POST', body: JSON.stringify(body) }),
  planProgress: () => call<PlanProgress>('/write/plan/progress'),
  writeArm: (acks: string[], version: string) => call<WriteStatus>('/write/arm', { method: 'POST', body: JSON.stringify({ acks, version }) }),
  writeDisarm: () => call<WriteStatus>('/write/disarm', { method: 'POST', body: '{}' }),
  heldOptions: (codes: string[]) =>
    call<{ courses: { code: string; row: number | null; course_key: unknown; sections: unknown; via: string; ms: number }[]; failed: { code: string; error: string }[]; stats: { workers: number; ms: number; parallel: boolean } }>('/write/held_options', { method: 'POST', body: JSON.stringify({ codes }) }),
  writeRead: <T>(tool: string, args: Record<string, number | string> = {}) => call<T>('/write/read', { method: 'POST', body: JSON.stringify({ tool, args }) }),
  writePreview: (tool: string, args: Record<string, number | string>, expect?: string[], check?: SeatCheckReq, credits?: PlanCreditsReq) =>
    call<Preview>('/write/preview', { method: 'POST', body: JSON.stringify({ tool, args, expect, check, credits }) }),
  writeConfirm: (token: string, phrase?: string) => call<Outcome>('/write/confirm', { method: 'POST', body: JSON.stringify({ token, phrase }) }),
  /** `known` maps course code to a key remembered from an earlier fetch; the server verifies every use of it. */
  sections: (rows: number[], known: Record<string, unknown> = {}) =>
    call<{ courses: unknown[]; stats?: FetchStats }>('/sections', { method: 'POST', body: JSON.stringify({ rows, known }) }),
}
