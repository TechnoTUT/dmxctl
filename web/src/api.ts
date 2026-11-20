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
  if (!r.ok) throw new Error(j.error ?? String(r.status))
  return j as T
}

export const fetchState = () => call<PanelState>('/api/state')
export const sendCommand = (c: Command) => call<PanelState>('/api/cmd', c)
