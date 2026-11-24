export type Rgb = [number, number, number]

/** GET /api/state の応答。crates/backend/src/web.rs の state と一致させる */
export interface PanelState {
  bpm: number
  link_peers: number
  blackout: boolean
  master: number
  fading: boolean
  emergency: boolean
  auto: {
    enabled: boolean
    online: boolean
    manual: number | null
    phrase: string
    look: string
    energy: number
    preset: number
    presets: string[]
  }
  blt: string
  main: Rgb | null
  sub: Rgb | null
  colors: { name: string; label: string; rgb: Rgb }[]
  scenes: string[]
  last_scene: string
  phrases: { key: string; label: string }[]
  outputs: string[]
}

/** POST /api/cmd の操作。command の cmd と一致させる */
export type Command =
  | { cmd: 'blackout' | 'fade' | 'tap' | 'resync' | 'auto' | 'auto_resume' | 'swap' }
  | { cmd: 'flash'; on: boolean }
  | { cmd: 'master'; value: number }
  | { cmd: 'preset'; index: number }
  | { cmd: 'look'; phrase: string }
  | { cmd: 'color'; slot: 'main' | 'sub'; name: string }
  | { cmd: 'scene'; name: string }

/** 合言葉が要るとき、または違うときに投げる */
export class PinError extends Error {
  constructor() {
    super('pin')
  }
}

/** サーバーが操作を断った。接続はできている */
export class ApiError extends Error {}

const PIN_KEY = 'dmxpin'

export function loadPin(): string {
  try {
    return localStorage.getItem(PIN_KEY) ?? ''
  } catch {
    return ''
  }
}

export function savePin(pin: string) {
  try {
    localStorage.setItem(PIN_KEY, pin)
  } catch {
    // 保存できなくても、そのページの間は使える
  }
}

async function call<T>(path: string, body?: Command): Promise<T> {
  const r = await fetch(path, {
    method: body ? 'POST' : 'GET',
    headers: { 'Content-Type': 'application/json', 'X-Pin': loadPin() },
    body: body ? JSON.stringify(body) : undefined,
  })
  if (r.status === 401) throw new PinError()
  const j = await r.json()
  if (!r.ok) throw new ApiError(j.error ?? String(r.status))
  return j as T
}

export const fetchState = () => call<PanelState>('/api/state')
export const sendCommand = (c: Command) => call<PanelState>('/api/cmd', c)

/** /api/events が使えない。接続数の上限などで、1秒ごとの読み直しに切り替える */
export class StreamUnavailable extends Error {}

/** 様子の変化を受け取り続ける。切れたり止まったりすると投げる。
 * 合言葉を X-Pin ヘッダーで送るため、EventSource ではなく fetch で読む */
export async function streamState(onState: (s: PanelState) => void, signal: AbortSignal, silentMs: number): Promise<void> {
  // サーバーは10秒ごとに合図を送る。それが途絶えたら、切れたものとして扱う
  const watchdog = new AbortController()
  let timer: ReturnType<typeof setTimeout> | undefined
  const arm = () => {
    clearTimeout(timer)
    timer = setTimeout(() => watchdog.abort(), silentMs)
  }
  const stop = () => watchdog.abort()
  signal.addEventListener('abort', stop)
  try {
    arm()
    const r = await fetch('/api/events', { headers: { 'X-Pin': loadPin() }, signal: watchdog.signal })
    if (r.status === 401) throw new PinError()
    if (!r.ok || !r.body) throw new StreamUnavailable()
    const reader = r.body.getReader()
    const decoder = new TextDecoder()
    let buf = ''
    for (;;) {
      const { done, value } = await reader.read()
      if (done) return
      arm()
      buf += decoder.decode(value, { stream: true })
      let i
      while ((i = buf.indexOf('\n\n')) >= 0) {
        const block = buf.slice(0, i)
        buf = buf.slice(i + 2)
        const data = block.split('\n').filter((l) => l.startsWith('data:')).map((l) => l.slice(5).trim()).join('\n')
        if (data) onState(JSON.parse(data) as PanelState)
      }
    }
  } finally {
    clearTimeout(timer)
    signal.removeEventListener('abort', stop)
  }
}
