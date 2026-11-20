import { useEffect, useRef, useState } from 'react'
import { Command, PanelState, Rgb } from './api'

export type Send = (c: Command) => void

const rgb = (c: Rgb | null) => (c ? `rgb(${c[0]},${c[1]},${c[2]})` : 'transparent')
// 明るい色のボタンは黒い文字で
const ink = (c: Rgb) => (c[0] * 0.299 + c[1] * 0.587 + c[2] * 0.114 > 150 ? '#111' : '#fff')

function Choice({ label, on, onClick, style, className }: { label: string; on?: boolean; onClick: () => void; style?: React.CSSProperties; className?: string }) {
  return (
    <button className={[className, on ? 'on' : ''].filter(Boolean).join(' ')} style={style} onClick={onClick}>
      {label}
    </button>
  )
}

export function PinBox({ onSubmit }: { onSubmit: (pin: string) => void }) {
  const [pin, setPin] = useState('')
  return (
    <div className="card">
      <h2>合言葉</h2>
      <form className="row" onSubmit={(e) => { e.preventDefault(); onSubmit(pin) }}>
        <input type="password" placeholder="アプリで決めた合言葉" value={pin} onChange={(e) => setPin(e.target.value)} />
        <button type="submit">入る</button>
      </form>
    </div>
  )
}

export function Status({ s }: { s: PanelState }) {
  const a = s.auto
  const autoChip = !a.enabled ? 'オート OFF' : a.manual != null ? '✋ 手動優先' : a.online ? '● 曲を受信中' : '○ 待機(薄い水色)'
  const bad = s.outputs.filter((o) => o !== '無効' && /エラー|失敗|切断|開けません/.test(o)).length
  return (
    <>
      {s.emergency && <div className="card danger">安全停止中です。アプリを起動し直してください</div>}
      {a.enabled && s.blt.startsWith('⚠') && <div className="card warn">{s.blt}</div>}
      <div className="card">
        <div className="row">
          <span className={'chip' + (a.enabled && a.online && a.manual == null ? ' ok' : '')}>{autoChip}</span>
          <span className="chip">BPM {s.bpm.toFixed(1)}</span>
          <span className={'chip' + (s.link_peers > 0 ? ' ok' : '')}>Link {s.link_peers}</span>
          {a.phrase && <span className="chip">フレーズ {a.phrase}</span>}
          <span className={'chip ' + (bad ? 'bad' : 'ok')}>出力 {bad ? `エラー ${bad}` : 'OK'}</span>
        </div>
      </div>
    </>
  )
}

/** 全体光量。動かしている間は少し間を空けて送り、手を離すまでは受信した値で上書きしない */
function MasterSlider({ value, send }: { value: number; send: Send }) {
  const [drag, setDrag] = useState<number | null>(null)
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined)
  useEffect(() => () => clearTimeout(timer.current), [])
  const shown = drag ?? Math.round(value * 100)
  const change = (v: number) => {
    setDrag(v)
    clearTimeout(timer.current)
    timer.current = setTimeout(() => send({ cmd: 'master', value: v / 100 }), 60)
  }
  return (
    <div className="row loose">
      <span className="small">全体光量</span>
      <input type="range" min={0} max={100} step={1} value={shown} onChange={(e) => change(Number(e.target.value))} onPointerUp={() => setDrag(null)} onBlur={() => setDrag(null)} />
      <span className="master-value">{shown}%</span>
    </div>
  )
}

/** 押している間だけ点灯する */
function Flash({ send }: { send: Send }) {
  const [held, setHeld] = useState(false)
  const down = (e: React.PointerEvent) => { e.preventDefault(); setHeld(true); send({ cmd: 'flash', on: true }) }
  const up = (e: React.PointerEvent) => { if (!held) return; e.preventDefault(); setHeld(false); send({ cmd: 'flash', on: false }) }
  return (
    <button className={'big' + (held ? ' on' : '')} onPointerDown={down} onPointerUp={up} onPointerLeave={up} onPointerCancel={up}>
      FLASH
    </button>
  )
}

export function Master({ s, send }: { s: PanelState; send: Send }) {
  return (
    <div className="card">
      <div className="row">
        <Choice className="big blackout" label="BLACKOUT" on={s.blackout} onClick={() => send({ cmd: 'blackout' })} />
        <Choice className="big" label="暗転" on={s.fading} onClick={() => send({ cmd: 'fade' })} />
        <Flash send={send} />
      </div>
      <MasterSlider value={s.master} send={send} />
      <div className="row tight">
        <Choice label="TAP" onClick={() => send({ cmd: 'tap' })} />
        <Choice label="頭出し" onClick={() => send({ cmd: 'resync' })} />
        <span className="small">BPM {s.bpm.toFixed(1)}</span>
      </div>
    </div>
  )
}

export function Auto({ s, send }: { s: PanelState; send: Send }) {
  const a = s.auto
  const note = a.manual != null ? `手動優先 あと${Math.ceil(a.manual)}秒` : a.online ? `盛り上がり ${Math.round(a.energy * 100)}%` : ''
  return (
    <div className="card">
      <h2>オート</h2>
      <div className="row">
        <Choice label={a.enabled ? 'オート ON' : 'オート OFF'} on={a.enabled} onClick={() => send({ cmd: 'auto' })} />
        {a.manual != null && <Choice label="オートに戻す" onClick={() => send({ cmd: 'auto_resume' })} />}
        <span className="small">{note}</span>
      </div>
      <h2 className="gap">プリセット</h2>
      <div className="row seg">
        {a.presets.map((n, i) => <Choice key={i} label={n} on={i === a.preset} onClick={() => send({ cmd: 'preset', index: i })} />)}
      </div>
      <h2 className="gap">フレーズを試す</h2>
      <div className="grid">
        {s.phrases.map((p) => <Choice key={p.key} label={p.label} on={a.look === p.key} onClick={() => send({ cmd: 'look', phrase: p.key })} />)}
      </div>
    </div>
  )
}

export function Colors({ s, send }: { s: PanelState; send: Send }) {
  const [slot, setSlot] = useState<'main' | 'sub'>('main')
  return (
    <div className="card">
      <h2>色 <span className="small">選ぶと、しばらくオートより優先</span></h2>
      <div className="row mb">
        <button className={slot === 'main' ? 'on' : ''} onClick={() => setSlot('main')}>メイン <span className="swatch" style={{ background: rgb(s.main) }} /></button>
        <button className={slot === 'sub' ? 'on' : ''} onClick={() => setSlot('sub')}>サブ <span className="swatch" style={{ background: rgb(s.sub) }} /></button>
        <Choice label="入れ替え" onClick={() => send({ cmd: 'swap' })} />
      </div>
      <div className="grid">
        {s.colors.map((c) => <Choice key={c.name} className="color" label={c.label} style={{ background: rgb(c.rgb), color: ink(c.rgb) }} onClick={() => send({ cmd: 'color', slot, name: c.name })} />)}
      </div>
    </div>
  )
}

export function Scenes({ s, send }: { s: PanelState; send: Send }) {
  if (!s.scenes.length) return null
  return (
    <div className="card">
      <h2>シーン</h2>
      <div className="grid">
        {s.scenes.map((n) => <Choice key={n} label={n} on={s.last_scene === n} onClick={() => send({ cmd: 'scene', name: n })} />)}
      </div>
    </div>
  )
}
