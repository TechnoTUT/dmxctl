import { AlertTriangle, OctagonAlert } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { Command, PanelState, Rgb } from './api'

export type Send = (c: Command) => void

const rgb = (c: Rgb | null) => (c ? `rgb(${c[0]},${c[1]},${c[2]})` : 'transparent')
// 明るい色のボタンは黒い文字で
const ink = (c: Rgb) => (c[0] * 0.299 + c[1] * 0.587 + c[2] * 0.114 > 150 ? '#111' : '#fff')

const btn = 'whitespace-nowrap rounded-xl border px-3.5 py-3 text-sm font-semibold transition active:scale-[.97]'
// 並べるボタン。長い名前は、1行に入り切らないときだけ中で折り返す
const wrap = 'max-w-full min-w-24 grow !whitespace-normal [overflow-wrap:anywhere]'
const idle = 'border-slate-200 bg-slate-100 text-slate-700 hover:bg-slate-200 dark:border-slate-800 dark:bg-slate-900 dark:text-slate-200 dark:hover:bg-slate-800'
const active = 'border-[#C7000A] bg-[#C7000A] text-white shadow-md shadow-[#C7000A]/20'

function Card({ title, note, children, tone }: { title?: string; note?: string; children: React.ReactNode; tone?: 'warn' | 'danger' }) {
  const toneCls =
    tone === 'danger' ? 'border-rose-300 bg-rose-50 text-rose-700 dark:border-rose-900 dark:bg-rose-950/40 dark:text-rose-300'
    : tone === 'warn' ? 'border-amber-300 bg-amber-50 text-amber-700 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-300'
    : 'border-slate-200 bg-white dark:border-slate-800 dark:bg-slate-900'
  return (
    <section className={`mb-4 rounded-2xl border p-4 shadow-sm short:mb-3 short:p-3 ${toneCls}`}>
      {title && (
        <h2 className="mb-3 flex flex-wrap items-baseline gap-x-2 text-sm font-bold text-slate-500 dark:text-slate-400">
          <span className="whitespace-nowrap">{title}</span>
          {note && <span className="text-xs font-normal">{note}</span>}
        </h2>
      )}
      {children}
    </section>
  )
}

function Sub({ children }: { children: React.ReactNode }) {
  return <h3 className="mb-2 mt-4 short:mt-3 text-xs font-bold uppercase tracking-wider text-slate-400">{children}</h3>
}

function Choice({ label, on, onClick, style, className = '' }: { label: string; on?: boolean; onClick: () => void; style?: React.CSSProperties; className?: string }) {
  return (
    <button className={`${btn} ${on ? active : idle} ${className}`} style={style} onClick={onClick}>
      {label}
    </button>
  )
}

export function Notice({ text }: { text: string }) {
  return (
    <Card tone="warn">
      <p className="flex items-center gap-2"><AlertTriangle className="size-5 shrink-0" />{text}</p>
    </Card>
  )
}

export function PinBox({ onSubmit }: { onSubmit: (pin: string) => void }) {
  const [pin, setPin] = useState('')
  return (
    <Card title="合言葉">
      <form className="flex gap-2" onSubmit={(e) => { e.preventDefault(); onSubmit(pin) }}>
        <input
          type="password"
          placeholder="アプリで決めた合言葉"
          value={pin}
          onChange={(e) => setPin(e.target.value)}
          className="min-w-0 flex-1 rounded-xl border border-slate-200 bg-slate-100 px-3 py-2 dark:border-slate-800 dark:bg-slate-950"
        />
        <button type="submit" className={`${btn} ${active}`}>入る</button>
      </form>
    </Card>
  )
}

function Chip({ children, tone }: { children: React.ReactNode; tone?: 'ok' | 'bad' }) {
  const t = tone === 'ok' ? 'text-emerald-600 dark:text-emerald-400' : tone === 'bad' ? 'text-rose-600 dark:text-rose-400' : 'text-slate-500 dark:text-slate-400'
  return <span className={`whitespace-nowrap rounded-full border border-slate-200 bg-slate-100 px-2.5 py-1 text-xs font-semibold dark:border-slate-800 dark:bg-slate-950 ${t}`}>{children}</span>
}

export function Status({ s }: { s: PanelState }) {
  const a = s.auto
  const autoChip = !a.enabled ? 'オート OFF' : a.manual != null ? '✋ 手動優先' : a.online ? '● 曲を受信中' : '○ 待機(薄い水色)'
  const bad = s.outputs.filter((o) => o !== '無効' && /エラー|失敗|切断|開けません/.test(o)).length
  return (
    <>
      {s.emergency && (
        <Card tone="danger">
          <p className="flex items-center gap-2 font-semibold"><OctagonAlert className="size-5" />安全停止中です。アプリを起動し直してください</p>
        </Card>
      )}
      {a.enabled && s.blt.startsWith('⚠') && (
        <Card tone="warn">
          <p className="flex items-center gap-2"><AlertTriangle className="size-5 shrink-0" />{s.blt}</p>
        </Card>
      )}
      <Card>
        <div className="flex flex-wrap gap-2">
          <Chip tone={a.enabled && a.online && a.manual == null ? 'ok' : undefined}>{autoChip}</Chip>
          <Chip>BPM {s.bpm.toFixed(1)}</Chip>
          <Chip tone={s.link_peers > 0 ? 'ok' : undefined}>Link {s.link_peers}</Chip>
          {a.phrase && <Chip>フレーズ {a.phrase}</Chip>}
          <Chip tone={bad ? 'bad' : 'ok'}>出力 {bad ? `エラー ${bad}` : 'OK'}</Chip>
        </div>
      </Card>
    </>
  )
}

/** 全体光量。動かしている間と、離してから反映されるまでは、手元の値を見せる。
 * 受信した値で上書きすると、離した瞬間に古い値へ戻って見える */
function MasterSlider({ value, send }: { value: number; send: Send }) {
  const [drag, setDrag] = useState<number | null>(null)
  const held = useRef(false)
  const sendTimer = useRef<ReturnType<typeof setTimeout>>(undefined)
  const settleTimer = useRef<ReturnType<typeof setTimeout>>(undefined)
  useEffect(() => () => { clearTimeout(sendTimer.current); clearTimeout(settleTimer.current) }, [])
  const server = Math.round(value * 100)
  // 離したあとに、送った値が戻ってきたら手元の値をやめる
  useEffect(() => {
    if (drag !== null && !held.current && drag === server) setDrag(null)
  }, [drag, server])
  // 反映されなかったときも、しばらくしたら受信した値に戻す
  const settle = () => {
    clearTimeout(settleTimer.current)
    settleTimer.current = setTimeout(() => setDrag(null), 1500)
  }
  const change = (v: number) => {
    setDrag(v)
    clearTimeout(sendTimer.current)
    sendTimer.current = setTimeout(() => send({ cmd: 'master', value: v / 100 }), 60)
    if (!held.current) settle()
  }
  const shown = drag ?? server
  return (
    <div className="mt-4 flex items-center gap-3 short:mt-3">
      <span className="whitespace-nowrap text-xs font-semibold text-slate-500 dark:text-slate-400">全体光量</span>
      <input
        type="range" min={0} max={100} step={1} value={shown}
        className="h-8 flex-1 accent-[#C7000A]"
        onPointerDown={() => { held.current = true; clearTimeout(settleTimer.current) }}
        onPointerUp={() => { held.current = false; settle() }}
        onPointerCancel={() => { held.current = false; settle() }}
        onChange={(e) => change(Number(e.target.value))}
      />
      <span className="w-11 text-right font-mono text-sm font-bold tabular-nums">{shown}%</span>
    </div>
  )
}

/** 押している間、FLASH を送り直す間隔。サーバーは最後に届いてから 1.5 秒で消す(crates/backend/src/web.rs の WEB_FLASH_HOLD) */
const FLASH_RESEND_MS = 500

/** 押している間だけ点灯する */
function Flash({ send }: { send: Send }) {
  const [held, setHeld] = useState(false)
  // 押している間は送り続ける。離した合図が届かなくても(電波切れ・タブを閉じた)、途絶えればサーバー側で消える
  useEffect(() => {
    if (!held) return
    const t = setInterval(() => send({ cmd: 'flash', on: true }), FLASH_RESEND_MS)
    return () => clearInterval(t)
  }, [held, send])
  const down = (e: React.PointerEvent) => { e.preventDefault(); setHeld(true); send({ cmd: 'flash', on: true }) }
  const up = (e: React.PointerEvent) => { if (!held) return; e.preventDefault(); setHeld(false); send({ cmd: 'flash', on: false }) }
  return (
    <button className={`${btn} flex-1 py-5 short:py-3 ${held ? active : idle}`} onPointerDown={down} onPointerUp={up} onPointerLeave={up} onPointerCancel={up}>
      FLASH
    </button>
  )
}

export function Master({ s, send }: { s: PanelState; send: Send }) {
  return (
    <Card>
      <div className="flex gap-2">
        <button
          className={`${btn} flex-1 py-5 short:py-3 ${s.blackout ? 'border-rose-600 bg-rose-600 text-white' : idle}`}
          onClick={() => send({ cmd: 'blackout' })}
        >
          BLACKOUT
        </button>
        <Choice className="flex-1 py-5 short:py-3" label="暗転" on={s.fading} onClick={() => send({ cmd: 'fade' })} />
        <Flash send={send} />
      </div>
      <MasterSlider value={s.master} send={send} />
      <div className="mt-3 flex items-center gap-2">
        <Choice label="TAP" onClick={() => send({ cmd: 'tap' })} />
        <Choice label="頭出し" onClick={() => send({ cmd: 'resync' })} />
        <span className="whitespace-nowrap font-mono text-xs text-slate-500 dark:text-slate-400">BPM {s.bpm.toFixed(1)}</span>
      </div>
    </Card>
  )
}

export function Auto({ s, send }: { s: PanelState; send: Send }) {
  const a = s.auto
  const note = a.manual != null ? `手動優先 あと${Math.ceil(a.manual)}秒` : a.online ? `盛り上がり ${Math.round(a.energy * 100)}%` : ''
  return (
    <Card title="オート">
      <div className="flex flex-wrap items-center gap-2">
        <Choice label={a.enabled ? 'オート ON' : 'オート OFF'} on={a.enabled} onClick={() => send({ cmd: 'auto' })} />
        {a.manual != null && <Choice label="オートに戻す" onClick={() => send({ cmd: 'auto_resume' })} />}
        <span className="whitespace-nowrap text-xs text-slate-500 dark:text-slate-400">{note}</span>
      </div>
      <Sub>プリセット</Sub>
      <div className="flex flex-wrap gap-2">
        {a.presets.map((n, i) => <Choice key={i} className="min-w-24 max-w-full flex-1 !whitespace-normal [overflow-wrap:anywhere]" label={n} on={i === a.preset} onClick={() => send({ cmd: 'preset', index: i })} />)}
      </div>
      <Sub>フレーズを試す</Sub>
      <div className="flex flex-wrap gap-2">
        {s.phrases.map((p) => <Choice key={p.key} className={wrap} label={p.label} on={a.look === p.key} onClick={() => send({ cmd: 'look', phrase: p.key })} />)}
      </div>
    </Card>
  )
}

export function Colors({ s, send }: { s: PanelState; send: Send }) {
  const [slot, setSlot] = useState<'main' | 'sub'>('main')
  const slotBtn = (id: 'main' | 'sub', label: string, c: Rgb | null) => (
    <button className={`${btn} flex items-center gap-2 ${slot === id ? active : idle}`} onClick={() => setSlot(id)}>
      {label}
      <span className="size-6 rounded-md border border-white/30" style={{ background: rgb(c) }} />
    </button>
  )
  return (
    <Card title="色" note="選ぶと、しばらくオートより優先">
      <div className="mb-3 flex flex-wrap gap-2">
        {slotBtn('main', 'メイン', s.main)}
        {slotBtn('sub', 'サブ', s.sub)}
        <Choice label="入れ替え" onClick={() => send({ cmd: 'swap' })} />
      </div>
      <div className="flex flex-wrap gap-2">
        {s.colors.map((c) => (
          <button
            key={c.name}
            className="max-w-full min-w-24 grow whitespace-normal rounded-xl border-2 border-transparent px-3 py-2.5 text-[13px] [overflow-wrap:anywhere] font-semibold transition active:scale-[.97]"
            style={{ background: rgb(c.rgb), color: ink(c.rgb) }}
            onClick={() => send({ cmd: 'color', slot, name: c.name })}
          >
            {c.label}
          </button>
        ))}
      </div>
    </Card>
  )
}

export function Scenes({ s, send }: { s: PanelState; send: Send }) {
  if (!s.scenes.length) return null
  return (
    <Card title="シーン">
      <div className="flex flex-wrap gap-2">
        {s.scenes.map((n) => <Choice key={n} className={wrap} label={n} on={s.last_scene === n} onClick={() => send({ cmd: 'scene', name: n })} />)}
      </div>
    </Card>
  )
}
