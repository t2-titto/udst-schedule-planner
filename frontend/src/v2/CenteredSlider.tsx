import type { CSSProperties } from 'react'
import { formatScore } from './scale'
import './preferences.css'

export interface CenteredSliderProps {
  /** Page scale, -5 to +5. */
  value: number
  onChange: (value: number) => void
  label: string
  id: string
}

const MIN = -5
const MAX = 5

/** A -5..+5 slider whose filled track runs from the centre to the knob. */
export function CenteredSlider({ value, onChange, label, id }: CenteredSliderProps) {
  const v = Math.min(MAX, Math.max(MIN, value))
  const p = (v - MIN) / (MAX - MIN)
  const style = { '--p': p, '--lo': Math.min(p, 0.5), '--hi': Math.max(p, 0.5) } as CSSProperties
  return (
    <div className="pref-row">
      <label className="pref-row__label" htmlFor={id}>
        {label}
      </label>
      <span className="pref-row__end" aria-hidden="true">-5</span>
      <div className="pref-slider" style={style}>
        <output className="pref-slider__bubble" htmlFor={id} aria-hidden="true">
          {formatScore(v)}
        </output>
        <input
          id={id}
          className="pref-range"
          type="range"
          min={MIN}
          max={MAX}
          step={0.5}
          value={v}
          aria-valuetext={formatScore(v)}
          onChange={(e) => onChange(Number(e.currentTarget.value))}
        />
      </div>
      <span className="pref-row__end" aria-hidden="true">+5</span>
    </div>
  )
}
