import { useState } from 'react'

const SUBWAY_EMBED_URL = 'https://files.gamezhero.com/game/905/1d1/9051d1173be765fb/data/index.html'

export function SubwayRunner() {
  const [tall, setTall] = useState(false)

  return (
    <div className="subway-box">
      <div className="g2048-head">
        <span>Subway Surfers</span>
        <button type="button" className="btn btn-sm" onClick={() => setTall((x) => !x)}>
          {tall ? 'Compact' : 'Tall'}
        </button>
      </div>

      <div className={`subway-frame-wrap ${tall ? 'tall' : ''}`}>
        <iframe
          title="Subway Surfers HTML5 Game"
          src={SUBWAY_EMBED_URL}
          className="subway-iframe"
          sandbox="allow-scripts allow-same-origin allow-pointer-lock allow-popups"
          allowFullScreen
        />
      </div>
    </div>
  )
}
