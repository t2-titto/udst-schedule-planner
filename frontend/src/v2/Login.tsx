import { useEffect, useState, type FormEvent } from 'react'
import { api, ApiError, type ServerAccount, type Status } from '../api'
import type { ThemeMode } from './theme'

interface Props {
  onSignedIn: (status: Status) => void
  onSwitchAccount: (id: string) => void
  initialError?: string
  theme: ThemeMode
  onToggleTheme: () => void
}

export function Login({ onSignedIn, onSwitchAccount, initialError, theme, onToggleTheme }: Props) {
  const [user, setUser] = useState('')
  const [pass, setPass] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState(initialError ?? '')
  const [accounts, setAccounts] = useState<ServerAccount[]>([])

  useEffect(() => {
    setError(initialError ?? '')
  }, [initialError])

  useEffect(() => {
    let alive = true
    api.accounts().then((res) => {
      if (alive) setAccounts(res.accounts ?? [])
    }).catch(() => { /* ignore when not signed in yet */ })
    return () => { alive = false }
  }, [])

  useEffect(() => () => setPass(''), [])

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    if (!user.trim() || !pass || busy) return
    setBusy(true)
    setError('')
    try {
      const res = await api.login(user.trim(), pass)
      onSignedIn(res)
    } catch (err) {
      setError(err instanceof ApiError ? err.message : 'Sign-in failed. Please check your credentials.')
    } finally {
      setPass('')
      setBusy(false)
    }
  }

  return (
    <main className="login-shell">
      <section className="login-card">
        <div className="login-head">
          <div>
            <div className="brand-mark">
              <span className="brand-dot" aria-hidden="true" />
              <span>Planie</span>
            </div>
            <p className="login-sub">Your classes &amp; degree, in one calm view.</p>
          </div>
          <button
            type="button"
            className="icon-btn"
            aria-label={theme === 'dark' ? 'Switch to light mode' : 'Switch to dark mode'}
            onClick={onToggleTheme}
          >
            {theme === 'dark' ? '☀️ Light' : '🌙 Dark'}
          </button>
        </div>

        <form className="login-form" onSubmit={submit}>
          <label className="field-label">
            Student number
            <input
              type="text"
              autoComplete="username"
              placeholder="60xxxxxx"
              value={user}
              onChange={(e) => setUser(e.target.value)}
              disabled={busy}
            />
          </label>
          <label className="field-label">
            Password
            <input
              type="password"
              autoComplete="current-password"
              placeholder="PeopleSoft password"
              value={pass}
              onChange={(e) => setPass(e.target.value)}
              disabled={busy}
            />
          </label>

          {error ? <p className="status-badge bad" role="alert">{error}</p> : null}

          <button type="submit" className="btn btn-primary" disabled={busy || !user.trim() || !pass}>
            {busy ? 'Signing in…' : 'Sign in'}
          </button>

          <p className="calm-notice">Changes only happen when you press Confirm &amp; Enroll.</p>
        </form>

        {accounts.length > 0 ? (
          <div className="switch-row">
            <span className="pill-group-label">Or continue in this session</span>
            <div className="switch-chips">
              {accounts.map((a) => (
                <button
                  key={a.id}
                  type="button"
                  className="btn btn-sm"
                  onClick={() => onSwitchAccount(a.id)}
                >
                  {a.demo ? 'Demo' : a.userHint}
                </button>
              ))}
            </div>
          </div>
        ) : null}
      </section>
    </main>
  )
}
