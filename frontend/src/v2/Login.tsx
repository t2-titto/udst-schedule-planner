import { useEffect, useRef, useState } from 'react'
import { api, ApiError } from '../api'
import type { RamAccount } from './accounts'

interface Props {
  /** Already signed in on the server (it was started with saved credentials): only the consent is needed. */
  signedInAs: string | null
  ramAccounts?: RamAccount[]
  onSwitchAccount?: (acc: RamAccount) => Promise<void>
  onHelp?: () => void
  onDone: (info?: { userHint: string; username: string; password: string }) => void
}

/** The only login is PeopleSoft's: student number and password, and one line saying what applying does. */
export function Login({ signedInAs, ramAccounts = [], onSwitchAccount, onHelp, onDone }: Props) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  // What the server says applying does (it owns the text). The page shows every statement and asks for ONE acknowledgement; the same
  // ids and version are sent to the server when writes are switched on, so the text that was read is the text that was accepted.
  const [statements, setStatements] = useState<{ id: string; text: string }[] | null>(null)
  const [accepted, setAccepted] = useState(false)
  useEffect(() => {
    api.writeStatus().then((w) => setStatements(w.consent.statements)).catch(() => setStatements(null))
  }, [])
  const userRef = useRef<HTMLInputElement>(null)
  const passRef = useRef<HTMLInputElement>(null)

  const submit = async (e: React.FormEvent) => {
    e.preventDefault()
    setError(null)
    if (signedInAs) {
      onDone()
      return
    }
    const username = userRef.current?.value ?? ''
    const password = passRef.current?.value ?? ''
    if (passRef.current) passRef.current.value = '' // the password leaves the form field the moment it is sent
    setBusy(true)
    try {
      const r = await api.login(username, password)
      onDone({ userHint: r.userHint, username, password })
    } catch (err) {
      setError(err instanceof ApiError ? err.message : 'Could not sign in. Try again.')
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="v2">
      <header className="v2-bar">
        <h1>Schedule planner</h1>
        <span className="v2-spacer" />
        {onHelp ? <button type="button" className="v2-linkbtn" onClick={onHelp}>Tutorial</button> : null}
      </header>
      <div className="v2-center">
        <form className="v2-card" onSubmit={submit}>
          <h2>{signedInAs ? `Continue as ${signedInAs}` : 'Sign in with PeopleSoft'}</h2>
          {signedInAs ? null : (
            <>
              <div className="v2-field">
                <label htmlFor="v2-user">Student number</label>
                <input id="v2-user" ref={userRef} autoComplete="username" inputMode="numeric" required />
              </div>
              <div className="v2-field">
                <label htmlFor="v2-pass">Password</label>
                <input id="v2-pass" ref={passRef} type="password" autoComplete="current-password" required />
              </div>
            </>
          )}
          <p className="v2-muted">What you apply here changes your real PeopleSoft account.</p>
          {statements ? (
            <details open>
              <summary>What applying does ({statements.length} statements)</summary>
              <ul className="v2-consent">{statements.map((st) => <li key={st.id}>{st.text}</li>)}</ul>
            </details>
          ) : <p className="v2-muted">The statements could not be loaded, so signing in is off. Reload the page.</p>}
          <label className="v2-ack"><input type="checkbox" checked={accepted} onChange={(e) => setAccepted(e.target.checked)} /> I have read these and accept them</label>
          {error ? <p className="v2-alert" role="alert">{error}</p> : null}
          <button type="submit" className="btn" disabled={busy || !accepted || !statements}>{busy ? 'Signing in…' : signedInAs ? 'Continue' : 'Sign in'}</button>
          {!signedInAs && ramAccounts.length > 0 && onSwitchAccount ? (
            <div style={{ marginTop: 'var(--space-sm)' }}>
              <p className="v2-muted" style={{ marginBottom: '0.25rem' }}>Switch to an account in this tab's memory (RAM):</p>
              <div style={{ display: 'flex', gap: '0.5rem', flexWrap: 'wrap' }}>
                {ramAccounts.map((acc) => (
                  <button
                    key={acc.sessionId}
                    type="button"
                    className="btn btn-quiet btn-sm"
                    disabled={busy || !accepted || !statements}
                    onClick={() => {
                      setError(null)
                      setBusy(true)
                      void onSwitchAccount(acc).catch((err) => setError(err instanceof ApiError ? err.message : 'Could not switch account.')).finally(() => setBusy(false))
                    }}
                  >
                    {acc.userHint}
                  </button>
                ))}
              </div>
            </div>
          ) : null}
        </form>
      </div>
    </div>
  )
}
