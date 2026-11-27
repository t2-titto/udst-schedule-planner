import { Chessground } from '@lichess-org/chessground'
import type { Api as CgApi } from '@lichess-org/chessground/api'
import type { Key } from '@lichess-org/chessground/types'
import '@lichess-org/chessground/assets/chessground.base.css'
import '@lichess-org/chessground/assets/chessground.brown.css'
import '@lichess-org/chessground/assets/chessground.cburnett.css'
import { Chess } from 'chess.js'
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type PointerEvent } from 'react'
import { api } from '../api'
import {
  CHESS_CACHE_LIMIT,
  nextChessPuzzle,
  warmChessPuzzleCache,
  type ChessPuzzle,
} from './chessLogic'
import { SubwayRunner } from './SubwayRunner'

type Tab = '2048' | 'chess' | 'subway'
type Dir = 'up' | 'down' | 'left' | 'right'

const BEST_KEY = 'planie:2048:best'

// ---- Animated Tile-ID 2048 Engine ----
interface Tile2048 {
  id: number
  val: number
  row: number
  col: number
  prevRow?: number
  prevCol?: number
  isNew?: boolean
  merged?: boolean
}

interface Game2048State {
  tiles: Tile2048[]
  ghosts: Tile2048[]
  gain: number
  moved: boolean
}

let tileIdCounter = 1
const allocTileId = () => tileIdCounter++

function spawn2048Tile(tiles: Tile2048[]): Tile2048[] {
  const occupied = new Set(tiles.map((t) => `${t.row},${t.col}`))
  const empty: Array<[number, number]> = []
  for (let r = 0; r < 4; r++) {
    for (let c = 0; c < 4; c++) {
      if (!occupied.has(`${r},${c}`)) empty.push([r, c])
    }
  }
  if (empty.length === 0) return tiles
  const [row, col] = empty[Math.floor(Math.random() * empty.length)]
  return [...tiles, { id: allocTileId(), val: Math.random() < 0.9 ? 2 : 4, row, col, isNew: true }]
}

function init2048Game(): Game2048State {
  return {
    tiles: spawn2048Tile(spawn2048Tile([])),
    ghosts: [],
    gain: 0,
    moved: false,
  }
}

function step2048Tiles(prevTiles: Tile2048[], dir: Dir): Game2048State {
  const grid: (Tile2048 | null)[][] = Array.from({ length: 4 }, () => [null, null, null, null])
  for (const t of prevTiles) grid[t.row][t.col] = t

  const nextTiles: Tile2048[] = []
  const ghosts: Tile2048[] = []
  let gain = 0
  let moved = false

  for (let i = 0; i < 4; i++) {
    const coords: Array<[number, number]> = []
    for (let j = 0; j < 4; j++) {
      if (dir === 'left') coords.push([i, j])
      else if (dir === 'right') coords.push([i, 3 - j])
      else if (dir === 'up') coords.push([j, i])
      else coords.push([3 - j, i])
    }
    const line = coords.map(([r, c]) => grid[r][c]).filter((t): t is Tile2048 => t !== null)
    let targetIdx = 0
    for (let k = 0; k < line.length; k++) {
      const t1 = line[k]
      const [tr, tc] = coords[targetIdx++]
      if (k + 1 < line.length && line[k + 1].val === t1.val) {
        const t2 = line[++k]
        const mergedVal = t1.val * 2
        gain += mergedVal
        moved = true
        ghosts.push(
          { id: t1.id, val: t1.val, row: tr, col: tc, prevRow: t1.row, prevCol: t1.col },
          { id: t2.id, val: t2.val, row: tr, col: tc, prevRow: t2.row, prevCol: t2.col },
        )
        nextTiles.push({
          id: allocTileId(),
          val: mergedVal,
          row: tr,
          col: tc,
          merged: true,
        })
      } else {
        if (t1.row !== tr || t1.col !== tc) moved = true
        nextTiles.push({
          id: t1.id,
          val: t1.val,
          row: tr,
          col: tc,
          prevRow: t1.row,
          prevCol: t1.col,
        })
      }
    }
  }

  if (!moved) {
    return {
      tiles: prevTiles.map((t) => ({ id: t.id, val: t.val, row: t.row, col: t.col })),
      ghosts: [],
      gain: 0,
      moved: false,
    }
  }
  return {
    tiles: spawn2048Tile(nextTiles),
    ghosts,
    gain,
    moved: true,
  }
}

function hasMoves2048(tiles: Tile2048[]): boolean {
  if (tiles.length < 16) return true
  const grid: number[][] = Array.from({ length: 4 }, () => [0, 0, 0, 0])
  for (const t of tiles) grid[t.row][t.col] = t.val
  for (let r = 0; r < 4; r++) {
    for (let c = 0; c < 4; c++) {
      const v = grid[r][c]
      if ((r < 3 && grid[r + 1][c] === v) || (c < 3 && grid[r][c + 1] === v)) return true
    }
  }
  return false
}

// Expectimax + monotonic corner snake heuristic for smooth, strong 2048 AI play
const SNAKE_WEIGHTS = [
  [65536, 32768, 16384, 8192],
  [512, 1024, 2048, 4096],
  [256, 128, 64, 32],
  [2, 4, 8, 16],
]

function slideFastBoard(board: number[], dir: Dir): { next: number[]; moved: boolean; gain: number } {
  const next = new Array<number>(16).fill(0)
  let moved = false
  let gain = 0
  for (let i = 0; i < 4; i++) {
    const idxs: number[] = []
    for (let j = 0; j < 4; j++) {
      if (dir === 'left') idxs.push(i * 4 + j)
      else if (dir === 'right') idxs.push(i * 4 + (3 - j))
      else if (dir === 'up') idxs.push(j * 4 + i)
      else idxs.push((3 - j) * 4 + i)
    }
    const vals: number[] = []
    for (const idx of idxs) if (board[idx] > 0) vals.push(board[idx])
    let t = 0
    for (let k = 0; k < vals.length; k++) {
      const v = vals[k]
      const targetIdx = idxs[t++]
      if (k + 1 < vals.length && vals[k + 1] === v) {
        const mv = v * 2
        next[targetIdx] = mv
        gain += mv
        k++
      } else {
        next[targetIdx] = v
      }
    }
    for (const idx of idxs) if (next[idx] !== board[idx]) moved = true
  }
  return { next, moved, gain }
}

function eval2048Board(board: number[]): number {
  let score = 0
  let empties = 0
  let smooth = 0
  for (let r = 0; r < 4; r++) {
    for (let c = 0; c < 4; c++) {
      const v = board[r * 4 + c]
      if (v === 0) {
        empties++
        continue
      }
      score += v * SNAKE_WEIGHTS[r][c]
      if (c < 3 && board[r * 4 + c + 1] > 0) smooth -= Math.abs(Math.log2(v) - Math.log2(board[r * 4 + c + 1]))
      if (r < 3 && board[(r + 1) * 4 + c] > 0) smooth -= Math.abs(Math.log2(v) - Math.log2(board[(r + 1) * 4 + c]))
    }
  }
  return score + empties * 1200 + smooth * 400
}

function expectimax2048(board: number[], depth: number, playerTurn: boolean): number {
  if (depth === 0) return eval2048Board(board)
  if (playerTurn) {
    let best = -Infinity
    for (const dir of ['up', 'left', 'right', 'down'] as Dir[]) {
      const res = slideFastBoard(board, dir)
      if (!res.moved) continue
      const val = expectimax2048(res.next, depth - 1, false) + res.gain * 8
      if (val > best) best = val
    }
    return best === -Infinity ? -1e9 : best
  }
  const empty: number[] = []
  for (let i = 0; i < 16; i++) if (board[i] === 0) empty.push(i)
  if (empty.length === 0) return eval2048Board(board)
  // Sample up to 6 empty cells for fast, smooth frame budget
  const sample = empty.length <= 6 ? empty : empty.slice(0, 6)
  let total = 0
  for (const idx of sample) {
    board[idx] = 2
    total += 0.9 * expectimax2048(board, depth - 1, true)
    board[idx] = 4
    total += 0.1 * expectimax2048(board, depth - 1, true)
    board[idx] = 0
  }
  return total / sample.length
}

function pickBest2048Dir(tiles: Tile2048[]): Dir | null {
  const board = new Array<number>(16).fill(0)
  for (const t of tiles) board[t.row * 4 + t.col] = t.val
  let bestDir: Dir | null = null
  let bestScore = -Infinity
  const depth = board.filter((x) => x === 0).length <= 4 ? 4 : 2
  for (const dir of ['up', 'left', 'right', 'down'] as Dir[]) {
    const res = slideFastBoard(board, dir)
    if (!res.moved) continue
    const val = expectimax2048(res.next, depth, false) + res.gain * 12
    if (val > bestScore) {
      bestScore = val
      bestDir = dir
    }
  }
  return bestDir
}

function legalDestsOf(chess: Chess): Map<Key, Key[]> {
  const dests = new Map<Key, Key[]>()
  for (const m of chess.moves({ verbose: true })) {
    const arr = dests.get(m.from as Key) ?? []
    if (!arr.includes(m.to as Key)) arr.push(m.to as Key)
    dests.set(m.from as Key, arr)
  }
  return dests
}

export function WaitWidget({ title = 'While we get everything ready' }: { title?: string }) {
  const [tab, setTab] = useState<Tab>('2048')

  // ---- 2048 State & Refs ----
  const [g2048, setG2048] = useState<Game2048State>(() => init2048Game())
  const [score, setScore] = useState(0)
  const [best, setBest] = useState(() => {
    try { return Number(window.localStorage.getItem(BEST_KEY) || '0') || 0 } catch { return 0 }
  })
  const [aiMode, setAiMode] = useState(false)
  const board2048Ref = useRef<HTMLDivElement | null>(null)
  const tileElsRef = useRef(new Map<number, HTMLDivElement>())
  const swipeStartRef = useRef<{ x: number; y: number } | null>(null)
  const tilesRef = useRef<Tile2048[]>(g2048.tiles)
  tilesRef.current = g2048.tiles

  const step2048 = useCallback((dir: Dir) => {
    setG2048((prev) => {
      const next = step2048Tiles(prev.tiles, dir)
      if (next.moved && next.gain > 0) {
        setScore((s) => {
          const ns = s + next.gain
          setBest((b) => {
            const nb = Math.max(b, ns)
            try { window.localStorage.setItem(BEST_KEY, String(nb)) } catch { /* ignore */ }
            return nb
          })
          return ns
        })
      }
      return next
    })
  }, [])

  // Real 2048 sliding, merge bounce, and spawn pop animations
  useLayoutEffect(() => {
    if (tab !== '2048') return
    const boardEl = board2048Ref.current
    if (!boardEl) return
    const rect = boardEl.getBoundingClientRect()
    const cellStepX = (rect.width - 11) / 4
    const cellStepY = (rect.height - 11) / 4

    for (const t of g2048.ghosts) {
      const el = tileElsRef.current.get(t.id)
      if (!el || typeof el.animate !== 'function' || t.prevRow == null || t.prevCol == null) continue
      const dx = (t.prevCol - t.col) * cellStepX
      const dy = (t.prevRow - t.row) * cellStepY
      el.animate(
        [
          { transform: `translate3d(${dx}px, ${dy}px, 0)`, opacity: 1 },
          { transform: 'translate3d(0px, 0px, 0)', opacity: 0 },
        ],
        { duration: 110, easing: 'cubic-bezier(0.22, 1, 0.36, 1)', fill: 'forwards' },
      )
    }

    for (const t of g2048.tiles) {
      const el = tileElsRef.current.get(t.id)
      if (!el || typeof el.animate !== 'function') continue
      if (t.isNew) {
        el.animate(
          [
            { transform: 'scale(0.15)', opacity: 0 },
            { transform: 'scale(1.06)', opacity: 1, offset: 0.72 },
            { transform: 'scale(1)', opacity: 1 },
          ],
          { duration: 145, delay: 70, easing: 'cubic-bezier(0.22, 1, 0.36, 1)', fill: 'backwards' },
        )
      } else if (t.merged) {
        el.animate(
          [
            { transform: 'scale(0.84)', opacity: 0.9 },
            { transform: 'scale(1.18)', opacity: 1, offset: 0.5 },
            { transform: 'scale(1)', opacity: 1 },
          ],
          { duration: 165, delay: 65, easing: 'cubic-bezier(0.22, 1, 0.36, 1)', fill: 'backwards' },
        )
      } else if (t.prevRow != null && t.prevCol != null && (t.prevRow !== t.row || t.prevCol !== t.col)) {
        const dx = (t.prevCol - t.col) * cellStepX
        const dy = (t.prevRow - t.row) * cellStepY
        el.animate(
          [
            { transform: `translate3d(${dx}px, ${dy}px, 0)` },
            { transform: 'translate3d(0px, 0px, 0)' },
          ],
          { duration: 115, easing: 'cubic-bezier(0.22, 1, 0.36, 1)' },
        )
      }
    }
  }, [g2048, tab])

  useEffect(() => {
    if (tab !== '2048') return
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null
      if (target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.tagName === 'SELECT')) return
      if (e.key === 'ArrowLeft' || e.key === 'a' || e.key === 'A') { e.preventDefault(); step2048('left') }
      else if (e.key === 'ArrowRight' || e.key === 'd' || e.key === 'D') { e.preventDefault(); step2048('right') }
      else if (e.key === 'ArrowUp' || e.key === 'w' || e.key === 'W') { e.preventDefault(); step2048('up') }
      else if (e.key === 'ArrowDown' || e.key === 's' || e.key === 'S') { e.preventDefault(); step2048('down') }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [tab, step2048])

  const onBoardPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    swipeStartRef.current = { x: e.clientX, y: e.clientY }
  }
  const onBoardPointerUp = (e: PointerEvent<HTMLDivElement>) => {
    const s = swipeStartRef.current
    swipeStartRef.current = null
    if (!s) return
    const dx = e.clientX - s.x
    const dy = e.clientY - s.y
    if (Math.max(Math.abs(dx), Math.abs(dy)) < 22) return
    if (Math.abs(dx) > Math.abs(dy)) step2048(dx > 0 ? 'right' : 'left')
    else step2048(dy > 0 ? 'down' : 'up')
  }

  // Autonomous 2048 AI loop (~1.5x natural pace) while AI mode is toggled on
  useEffect(() => {
    if (tab !== '2048' || !aiMode) return
    const timer = window.setInterval(() => {
      const dir = pickBest2048Dir(tilesRef.current)
      if (!dir) {
        setAiMode(false)
        return
      }
      step2048(dir)
    }, 280)
    return () => window.clearInterval(timer)
  }, [tab, aiMode, step2048])

  // ---- Lichess Real-Time Chess Puzzles + Chessground UI ----
  const [puzzle, setPuzzle] = useState<ChessPuzzle | null>(null)
  const [chessStatus, setChessStatus] = useState<'loading' | 'play' | 'solved'>('loading')
  const [chessMsg, setChessMsg] = useState('')
  const cgHostRef = useRef<HTMLDivElement | null>(null)
  const cgApiRef = useRef<CgApi | null>(null)
  const chessRef = useRef<Chess>(new Chess())
  const puzzleSessionRef = useRef<{ puzzle: ChessPuzzle | null; solIdx: number; side: 'white' | 'black'; lastMove?: [Key, Key] }>({
    puzzle: null,
    solIdx: 0,
    side: 'white',
  })
  const chessTimerRef = useRef<number | null>(null)
  const warmedRef = useRef(false)

  const syncBoard = useCallback((interactive: boolean) => {
    const cg = cgApiRef.current
    const sess = puzzleSessionRef.current
    const ch = chessRef.current
    if (!cg) return
    const sideColor = sess.side
    cg.set({
      fen: ch.fen(),
      orientation: sideColor,
      turnColor: ch.turn() === 'w' ? 'white' : 'black',
      lastMove: sess.lastMove,
      check: ch.inCheck(),
      drawable: { autoShapes: [] },
      movable: {
        free: false,
        color: interactive ? sideColor : undefined,
        dests: interactive ? legalDestsOf(ch) : new Map(),
        showDests: true,
      },
    })
  }, [])

  const onUserMove = useCallback((orig: Key, dest: Key) => {
    const sess = puzzleSessionRef.current
    const p = sess.puzzle
    const ch = chessRef.current
    if (!p) return
    const expected = p.solution[sess.solIdx] ?? ''
    const promo = expected.length === 5 && expected.startsWith(`${orig}${dest}`) ? expected[4] : 'q'

    const mv = ch.moves({ verbose: true }).find((m) => m.from === orig && m.to === dest)
    if (!mv) {
      syncBoard(true)
      return
    }
    const uci = `${orig}${dest}${mv.isPromotion() ? promo : ''}`
    if (uci === expected) {
      ch.move({ from: orig, to: dest, promotion: mv.isPromotion() ? promo : undefined })
      sess.lastMove = [orig, dest]
      const nextStep = sess.solIdx + 1
      if (nextStep >= p.solution.length) {
        sess.solIdx = nextStep
        setChessStatus('solved')
        setChessMsg('Puzzle solved! Loading next…')
        syncBoard(false)
        chessTimerRef.current = window.setTimeout(() => {
          void loadNextPuzzleRef.current()
        }, 900)
      } else {
        const reply = p.solution[nextStep]
        setChessMsg('Good move!')
        syncBoard(false)
        chessTimerRef.current = window.setTimeout(() => {
          const rf = reply.slice(0, 2) as Key
          const rt = reply.slice(2, 4) as Key
          const rPromo = reply.length === 5 ? reply[4] : undefined
          try {
            ch.move({ from: rf, to: rt, promotion: rPromo })
          } catch { /* ignore invalid reply */ }
          sess.lastMove = [rf, rt]
          const afterReply = nextStep + 1
          sess.solIdx = afterReply
          if (afterReply >= p.solution.length) {
            setChessStatus('solved')
            setChessMsg('Puzzle solved! Loading next…')
            syncBoard(false)
            chessTimerRef.current = window.setTimeout(() => {
              void loadNextPuzzleRef.current()
            }, 900)
          } else {
            setChessMsg('Your turn — keep finding the best move')
            syncBoard(true)
          }
        }, 300)
      }
    } else {
      setChessMsg('Not the best move — try again')
      syncBoard(true)
      cgApiRef.current?.setAutoShapes([{ orig: dest, brush: 'red' }])
    }
  }, [syncBoard])

  // Stable ref so Chessground stays mounted across moves and status transitions
  const onUserMoveRef = useRef(onUserMove)
  onUserMoveRef.current = onUserMove

  // Initialize Lichess Chessground instance when Chess tab is mounted
  useEffect(() => {
    if (tab !== 'chess') {
      cgApiRef.current?.destroy()
      cgApiRef.current = null
      return
    }
    const host = cgHostRef.current
    if (!host) return
    const cg = Chessground(host, {
      coordinates: true,
      highlight: { lastMove: true, check: true },
      draggable: { enabled: true, showGhost: true },
      selectable: { enabled: true },
      animation: { enabled: true, duration: 220 },
      movable: {
        free: false,
        showDests: true,
        events: { after: (orig, dest) => onUserMoveRef.current(orig, dest) },
      },
    })
    cgApiRef.current = cg
    if (puzzleSessionRef.current.puzzle) {
      syncBoard(puzzleSessionRef.current.solIdx < (puzzleSessionRef.current.puzzle.solution.length))
    }
    return () => {
      cg.destroy()
      cgApiRef.current = null
    }
  }, [tab, syncBoard])

  const applyNewPuzzle = useCallback((p: ChessPuzzle) => {
    if (chessTimerRef.current != null) window.clearTimeout(chessTimerRef.current)
    let ch: Chess
    try {
      ch = new Chess(p.fen)
    } catch {
      ch = new Chess('6k1/5ppp/8/8/8/8/5PPP/3R2K1 w - - 0 1')
    }
    chessRef.current = ch
    const side: 'white' | 'black' = ch.turn() === 'w' ? 'white' : 'black'
    const lm: [Key, Key] | undefined =
      p.lastMove && p.lastMove.length >= 4
        ? [p.lastMove.slice(0, 2) as Key, p.lastMove.slice(2, 4) as Key]
        : undefined
    puzzleSessionRef.current = { puzzle: p, solIdx: 0, side, lastMove: lm }
    setPuzzle(p)
    setChessStatus('play')
    setChessMsg(side === 'white' ? 'White to find the best move' : 'Black to find the best move')
    syncBoard(true)
  }, [syncBoard])

  const loadNextPuzzleRef = useRef<() => Promise<void>>(async () => {})

  const loadNextPuzzle = useCallback(async () => {
    setChessStatus('loading')
    const p = await nextChessPuzzle(() => api.chessPuzzle())
    applyNewPuzzle(p)
    void warmChessPuzzleCache(() => api.chessPuzzle(), CHESS_CACHE_LIMIT)
  }, [applyNewPuzzle])
  loadNextPuzzleRef.current = loadNextPuzzle

  useEffect(() => () => {
    if (chessTimerRef.current != null) window.clearTimeout(chessTimerRef.current)
  }, [])

  useEffect(() => {
    if (warmedRef.current) return
    warmedRef.current = true
    void warmChessPuzzleCache(() => api.chessPuzzle(), CHESS_CACHE_LIMIT)
  }, [])

  useEffect(() => {
    if (tab === 'chess' && !puzzle) {
      void loadNextPuzzle()
    }
  }, [tab, puzzle, loadNextPuzzle])

  const noMovesLeft2048 = !hasMoves2048(g2048.tiles)

  return (
    <section className="wait-card" aria-label="Interactive waiting companion">
      <div className="suggestions-head">
        <h3>{title}</h3>
      </div>
      <div className="wait-tabs" role="tablist">
        <button type="button" role="tab" aria-selected={tab === '2048'} className={`wait-tab ${tab === '2048' ? 'active' : ''}`} onClick={() => setTab('2048')}>
          2048
        </button>
        <button type="button" role="tab" aria-selected={tab === 'chess'} className={`wait-tab ${tab === 'chess' ? 'active' : ''}`} onClick={() => setTab('chess')}>
          Chess
        </button>
        <button type="button" role="tab" aria-selected={tab === 'subway'} className={`wait-tab ${tab === 'subway' ? 'active' : ''}`} onClick={() => setTab('subway')}>
          Subway
        </button>
      </div>

      {tab === '2048' ? (
        <>
          <div className="g2048-head">
            <span>Score: <strong className="num">{score}</strong> · Best: <strong className="num">{best}</strong></span>
            <div className="pill-row">
              <button
                type="button"
                aria-pressed={aiMode}
                className={`btn btn-sm ${aiMode ? 'btn-primary' : ''}`}
                onClick={() => setAiMode((x) => !x)}
              >
                {aiMode ? 'Stop AI' : 'AI mode'}
              </button>
              <button
                type="button"
                className="btn btn-sm"
                onClick={() => { setAiMode(false); setG2048(init2048Game()); setScore(0) }}
              >
                New game
              </button>
            </div>
          </div>
          {noMovesLeft2048 ? (
            <div className="status-badge warn">No moves left — tap New game to play again</div>
          ) : null}
          <div
            ref={board2048Ref}
            className="g2048-board"
            onPointerDown={onBoardPointerDown}
            onPointerUp={onBoardPointerUp}
          >
            <div className="g2048-bg-grid" aria-hidden="true">
              {Array.from({ length: 16 }).map((_, i) => (
                <div key={i} className="g2048-slot" />
              ))}
            </div>
            <div className="g2048-tile-layer">
              {g2048.ghosts.map((t) => (
                <div
                  key={`g-${t.id}`}
                  ref={(el) => {
                    if (el) tileElsRef.current.set(t.id, el)
                    else tileElsRef.current.delete(t.id)
                  }}
                  aria-hidden="true"
                  className={`g2048-tile g2048-ghost g2048-r-${t.row} g2048-c-${t.col} g2048-v-${Math.min(t.val, 2048)}`}
                >
                  {t.val}
                </div>
              ))}
              {g2048.tiles.map((t) => (
                <div
                  key={t.id}
                  ref={(el) => {
                    if (el) tileElsRef.current.set(t.id, el)
                    else tileElsRef.current.delete(t.id)
                  }}
                  className={`g2048-tile g2048-r-${t.row} g2048-c-${t.col} g2048-v-${Math.min(t.val, 2048)}`}
                >
                  {t.val}
                </div>
              ))}
            </div>
          </div>
          <div className="dpad-row">
            <button type="button" className="btn btn-sm" aria-label="Move left" onClick={() => step2048('left')}>←</button>
            <button type="button" className="btn btn-sm" aria-label="Move up" onClick={() => step2048('up')}>↑</button>
            <button type="button" className="btn btn-sm" aria-label="Move down" onClick={() => step2048('down')}>↓</button>
            <button type="button" className="btn btn-sm" aria-label="Move right" onClick={() => step2048('right')}>→</button>
          </div>
        </>
      ) : null}

      {tab === 'chess' ? (
        <div className="chess-box">
          <div className="g2048-head">
            <div className="pill-row">
              {puzzle ? <span className="status-badge accent num">★ {puzzle.rating}</span> : null}
              <span className="status-badge ok">Lichess</span>
            </div>
            <div className="pill-row">
              {puzzle && chessStatus === 'play' ? (
                <button
                  type="button"
                  className="btn btn-sm"
                  onClick={() => {
                    const sess = puzzleSessionRef.current
                    const exp = puzzle.solution[sess.solIdx]
                    if (exp && exp.length >= 4 && cgApiRef.current) {
                      cgApiRef.current.setAutoShapes([{
                        orig: exp.slice(0, 2) as Key,
                        dest: exp.slice(2, 4) as Key,
                        brush: 'green',
                      }])
                    }
                  }}
                >
                  Hint
                </button>
              ) : null}
              <button type="button" className="btn btn-sm btn-primary" onClick={() => void loadNextPuzzle()}>
                {chessStatus === 'loading' ? 'Loading…' : 'Next puzzle'}
              </button>
            </div>
          </div>

          <div className={`chess-status-line ${chessStatus === 'solved' ? 'ok' : ''}`}>
            {chessStatus === 'loading' ? 'Fetching fresh Lichess tactical puzzle…' : chessMsg}
          </div>

          <div ref={cgHostRef} className="cg-host is2d" aria-label="Lichess Chessboard" />

          {puzzle?.themes?.length ? (
            <div className="pill-row">
              {puzzle.themes.slice(0, 4).map((t) => (
                <span key={t} className="status-badge">{t}</span>
              ))}
            </div>
          ) : null}
        </div>
      ) : null}

      {tab === 'subway' ? <SubwayRunner /> : null}
    </section>
  )
}
