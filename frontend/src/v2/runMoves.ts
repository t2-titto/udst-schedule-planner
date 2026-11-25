import { api, ApiError } from '../api'
import { moveRequest, type Move } from './savePlan'

export type Step = 'waiting' | 'working' | 'saved' | 'failed' | 'not sent'
export const msg = (e: unknown) => (e instanceof ApiError ? e.message : e instanceof Error ? e.message : 'Something went wrong.')

/**
 * Send plan moves to PeopleSoft one at a time, each as a preview then a confirm (course planning only). Writes are switched on for this
 * and off again afterwards, whatever happens. Stops at the first failure: the moves after it are not sent.
 */
export async function executeMoves(moves: Move[], onStep: (i: number, s: Step) => void): Promise<{ done: Move[]; error: string | null }> {
  const done: Move[] = []
  let error: string | null = null
  try {
    const ws = await api.writeStatus()
    if (ws.confirmMode === 'phrase') throw new ApiError('This server asks for a typed phrase for every write, which this screen does not support yet. Nothing was sent.', 0)
    if (!ws.armed) await api.writeArm(ws.consent.statements.map((s) => s.id), ws.consent.version)
    for (let i = 0; i < moves.length; i++) {
      const m = moves[i]
      onStep(i, 'working')
      try {
        const { args, credits, expect } = moveRequest(m)
        const pre = await api.writePreview('preview_plan_term', args, expect, undefined, credits)
        await api.writeConfirm(pre.token)
        onStep(i, 'saved')
        done.push(m)
      } catch (e) {
        onStep(i, 'failed')
        for (let j = i + 1; j < moves.length; j++) onStep(j, 'not sent')
        error = `${m.course.code}: ${msg(e)}`
        break
      }
    }
  } catch (e) {
    error = msg(e)
  }
  try { await api.writeDisarm() } catch { /* the idle timer switches them off anyway */ }
  return { done, error }
}
