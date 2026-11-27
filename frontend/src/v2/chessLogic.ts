export interface ChessPuzzle {
  id: string
  rating: number
  fen: string
  lastMove?: string
  solution: string[]
  themes?: string[]
  source?: 'live' | 'cached' | 'offline'
}

export interface ChessBoardState {
  board: (string | null)[][]
  turn: 'w' | 'b'
}

export const CHESS_CACHE_LIMIT = 10
const CHESS_STORAGE_KEY = 'planie:chess:cache:v1'

const FALLBACK_PUZZLES: ChessPuzzle[] = [
  {
    id: 'seed1',
    rating: 1120,
    fen: '6k1/5ppp/8/8/8/8/5PPP/3R2K1 w - - 0 1',
    solution: ['d1d8'],
    themes: ['backRankMate', 'mateIn1'],
    source: 'offline',
  },
  {
    id: 'seed2',
    rating: 1340,
    fen: 'r2qkb1r/ppp2ppp/2n1bn2/3pp3/4P3/2NP1N2/PPP1BPPP/R1BQK2R w KQkq - 0 6',
    solution: ['e4d5', 'f6d5', 'c3d5'],
    themes: ['advantage', 'short'],
    source: 'offline',
  },
  {
    id: 'seed3',
    rating: 1450,
    fen: 'r1bqk2r/pppp1ppp/2n2n2/2b1p3/2B1P3/3P1N2/PPP2PPP/RNBQK2R w KQkq - 0 5',
    lastMove: 'c8f5',
    solution: ['c4f7', 'e8f7', 'f3g5'],
    themes: ['fork', 'sacrifice'],
    source: 'offline',
  },
]

const memCache: ChessPuzzle[] = []
const playedIds = new Set<string>()
let storageHydrated = false

function hydrateCacheFromStorage(): void {
  if (storageHydrated) return
  storageHydrated = true
  try {
    const raw = window.localStorage.getItem(CHESS_STORAGE_KEY)
    if (!raw) return
    const parsed = JSON.parse(raw) as unknown
    if (!Array.isArray(parsed)) return
    for (const p of parsed) {
      if (
        p &&
        typeof p === 'object' &&
        typeof (p as ChessPuzzle).id === 'string' &&
        typeof (p as ChessPuzzle).fen === 'string' &&
        Array.isArray((p as ChessPuzzle).solution) &&
        (p as ChessPuzzle).solution.length > 0 &&
        memCache.length < CHESS_CACHE_LIMIT &&
        !memCache.some((m) => m.id === (p as ChessPuzzle).id)
      ) {
        memCache.push({ ...(p as ChessPuzzle), source: 'cached' })
      }
    }
  } catch { /* ignore storage errors */ }
}

function persistCache(): void {
  try {
    window.localStorage.setItem(CHESS_STORAGE_KEY, JSON.stringify(memCache.slice(0, CHESS_CACHE_LIMIT)))
  } catch { /* ignore storage errors */ }
}

export function getCachedPuzzleCount(): number {
  hydrateCacheFromStorage()
  return memCache.length
}

export function clearChessPuzzleCache(): void {
  memCache.length = 0
  playedIds.clear()
  storageHydrated = false
}

export function pushPuzzleToCache(puzzle: ChessPuzzle): void {
  hydrateCacheFromStorage()
  if (!puzzle.id || !puzzle.fen || puzzle.solution.length === 0) return
  if (memCache.some((p) => p.id === puzzle.id)) return
  memCache.unshift({ ...puzzle, source: puzzle.source ?? 'live' })
  if (memCache.length > CHESS_CACHE_LIMIT) {
    memCache.length = CHESS_CACHE_LIMIT
  }
  persistCache()
}

export async function warmChessPuzzleCache(
  fetcher: () => Promise<ChessPuzzle>,
  targetCount = CHESS_CACHE_LIMIT,
): Promise<number> {
  hydrateCacheFromStorage()
  let tries = 0
  const maxTries = Math.max(targetCount * 2, 12)
  while (memCache.length < targetCount && tries < maxTries) {
    tries++
    try {
      const p = await fetcher()
      if (p?.id && p?.fen && Array.isArray(p.solution) && p.solution.length > 0) {
        pushPuzzleToCache({ ...p, source: 'live' })
      }
      if (memCache.length < targetCount) {
        await new Promise((r) => setTimeout(r, 250))
      }
    } catch {
      break
    }
  }
  return memCache.length
}

/**
 * Fetch a fresh puzzle from Lichess when online (keeping a 10-puzzle cache in memory + localStorage).
 * Falls back cleanly to cached puzzles if offline or unreachable so puzzles never repeat unnecessarily.
 */
export async function nextChessPuzzle(fetcher: () => Promise<ChessPuzzle>): Promise<ChessPuzzle> {
  hydrateCacheFromStorage()

  // First try fetching a brand new puzzle live in realtime
  let attempts = 0
  while (attempts < 2) {
    attempts++
    try {
      const live = await fetcher()
      if (live?.id && live?.fen && Array.isArray(live.solution) && live.solution.length > 0) {
        if (playedIds.has(live.id) && attempts < 2) continue
        playedIds.add(live.id)
        pushPuzzleToCache({ ...live, source: 'live' })
        return { ...live, source: 'live' }
      }
    } catch {
      break
    }
  }

  // Offline / rate-limited fallback: pull next unplayed puzzle from our 10-puzzle cache
  const unplayed = memCache.find((p) => !playedIds.has(p.id))
  if (unplayed) {
    playedIds.add(unplayed.id)
    return { ...unplayed, source: 'cached' }
  }
  if (memCache.length > 0) {
    // Rotate cleanly once all 10 cached puzzles were played while offline
    for (const p of memCache) playedIds.delete(p.id)
    const recycled = memCache[0]
    playedIds.add(recycled.id)
    return { ...recycled, source: 'cached' }
  }
  const idx = playedIds.size % FALLBACK_PUZZLES.length
  const fb = FALLBACK_PUZZLES[idx]
  playedIds.add(fb.id)
  return fb
}

export function parseFen(fen: string): ChessBoardState {
  const parts = fen.trim().split(/\s+/)
  const rows = (parts[0] ?? '8/8/8/8/8/8/8/8').split('/')
  const board: (string | null)[][] = []
  for (let r = 0; r < 8; r++) {
    const row: (string | null)[] = []
    const src = rows[r] ?? '8'
    for (const ch of src) {
      if (ch >= '1' && ch <= '8') {
        for (let k = 0; k < Number(ch); k++) row.push(null)
      } else {
        row.push(ch)
      }
    }
    while (row.length < 8) row.push(null)
    board.push(row.slice(0, 8))
  }
  return { board, turn: parts[1] === 'b' ? 'b' : 'w' }
}

export function sqToCoords(sq: string): [number, number] | null {
  if (!/^[a-h][1-8]$/.test(sq)) return null
  const col = sq.charCodeAt(0) - 97
  const row = 8 - Number(sq[1])
  return [row, col]
}

export function coordsToSq(row: number, col: number): string {
  return `${String.fromCharCode(97 + col)}${8 - row}`
}

export function pieceColor(piece: string | null): 'w' | 'b' | null {
  if (!piece) return null
  return piece === piece.toUpperCase() ? 'w' : 'b'
}

export function applyUciMove(
  board: (string | null)[][],
  uci: string,
): { nextBoard: (string | null)[][]; from: [number, number]; to: [number, number] } | null {
  const from = sqToCoords(uci.slice(0, 2))
  const to = sqToCoords(uci.slice(2, 4))
  if (!from || !to) return null
  const [fr, fc] = from
  const [tr, tc] = to
  const piece = board[fr][fc]
  if (!piece) return null

  const next = board.map((r) => [...r])
  const promo = uci[4]
  let placed = piece
  if (promo && (piece === 'P' || piece === 'p')) {
    placed = piece === 'P' ? promo.toUpperCase() : promo.toLowerCase()
  }

  // En passant: diagonal pawn capture to empty square
  if ((piece === 'P' || piece === 'p') && fc !== tc && !next[tr][tc]) {
    next[fr][tc] = null
  }

  // Castling: king slides 2 files -> slide rook alongside
  if ((piece === 'K' || piece === 'k') && Math.abs(tc - fc) === 2 && fr === tr) {
    if (tc === 6) {
      next[tr][5] = next[tr][7]
      next[tr][7] = null
    } else if (tc === 2) {
      next[tr][3] = next[tr][0]
      next[tr][0] = null
    }
  }

  next[tr][tc] = placed
  next[fr][fc] = null
  return { nextBoard: next, from, to }
}

export function buildMoveUci(
  fr: number,
  fc: number,
  tr: number,
  tc: number,
  board: (string | null)[][],
  expectedUci?: string,
): string {
  const piece = board[fr][fc]
  const base = `${coordsToSq(fr, fc)}${coordsToSq(tr, tc)}`
  if ((piece === 'P' && tr === 0) || (piece === 'p' && tr === 7)) {
    const expPromo = expectedUci && expectedUci.startsWith(base) && expectedUci[4] ? expectedUci[4] : 'q'
    return `${base}${expPromo}`
  }
  return base
}
