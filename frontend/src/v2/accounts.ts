/**
 * Accounts signed into in this browser tab, kept strictly in JS memory (browser RAM) so switching accounts
 * is instant and credentials never touch localStorage, sessionStorage, or disk.
 */
export interface RamAccount {
  sessionId: string
  userHint: string
  username: string
  password: string
}

let ramAccounts: RamAccount[] = []

export function listRamAccounts(): RamAccount[] {
  return [...ramAccounts]
}

export function saveRamAccount(acc: RamAccount): void {
  ramAccounts = [...ramAccounts.filter((a) => a.sessionId !== acc.sessionId && a.username !== acc.username), acc]
}

export function removeRamAccount(sessionId: string): void {
  ramAccounts = ramAccounts.filter((a) => a.sessionId !== sessionId)
}
