import { useCallback, useEffect, useState } from 'react'
import { ApiError, Command, PanelState, PinError, StreamUnavailable, fetchState, sendCommand, streamState } from './api'

/** 切れたあとに接続し直すまでの時間、および読み直しの間隔 */
const RETRY_MS = 1000
/** サーバーは10秒ごとに合図を送る。これだけ何も届かなければ切れたものとする */
const SILENT_MS = 30_000
/** ストリームを断られて読み直しに切り替えたあと、ストリームに戻れるか試すまでの時間 */
const STREAM_RETRY_MS = 30_000

/** 中断できる待ち。中断されたら待たずに戻る */
function sleep(ms: number, signal: AbortSignal) {
  return new Promise<void>((resolve) => {
    const t = setTimeout(resolve, ms)
    signal.addEventListener('abort', () => { clearTimeout(t); resolve() }, { once: true })
  })
}

/** 様子の変化を受け取り、操作を送る。操作の応答にも最新の様子が入っている。
 * ストリームが使えないサーバーでは、1秒ごとに読み直す */
export function usePanel() {
  const [state, setState] = useState<PanelState | null>(null)
  // 接続できているか。操作が断られただけでは変えない
  const [offline, setOffline] = useState(false)
  // 操作が断られたときの理由。次に操作が受け付けられるまで出す
  const [notice, setNotice] = useState('')
  const [needPin, setNeedPin] = useState(false)
  // 合言葉を入れたあとなどに、接続し直す
  const [attempt, setAttempt] = useState(0)

  const apply = useCallback((s: PanelState) => {
    setState(s)
    setOffline(false)
    setNeedPin(false)
  }, [])

  const fail = useCallback((e: unknown) => {
    if (e instanceof PinError) {
      setNeedPin(true)
    } else if (e instanceof ApiError) {
      setNotice(e.message)
    } else {
      setOffline(true)
    }
  }, [])

  const send = useCallback(
    async (c: Command) => {
      try {
        apply(await sendCommand(c))
        // 操作が受け付けられたときだけ消す。先に届いた状態の通知では消さない
        setNotice('')
      } catch (e) {
        fail(e)
      }
    },
    [apply, fail],
  )

  useEffect(() => {
    const ac = new AbortController()
    // 一定時間だけ1秒ごとに読み直す。そのあとストリームを試し直す
    const pollFor = async (ms: number) => {
      const end = Date.now() + ms
      while (!ac.signal.aborted && Date.now() < end) {
        try {
          apply(await fetchState())
        } catch (e) {
          fail(e)
        }
        await sleep(RETRY_MS, ac.signal)
      }
    }
    const run = async () => {
      while (!ac.signal.aborted) {
        try {
          await streamState(apply, ac.signal, SILENT_MS)
        } catch (e) {
          if (ac.signal.aborted) return
          if (e instanceof PinError) {
            // 合言葉を入れると attempt が変わり、ここからやり直す
            setNeedPin(true)
            return
          }
          if (e instanceof StreamUnavailable) {
            await pollFor(STREAM_RETRY_MS)
            continue
          }
          setOffline(true)
        }
        await sleep(RETRY_MS, ac.signal)
      }
    }
    run()
    return () => ac.abort()
  }, [apply, fail, attempt])

  const reconnect = useCallback(() => setAttempt((n) => n + 1), [])

  return { state, offline, notice, needPin, send, reconnect }
}
