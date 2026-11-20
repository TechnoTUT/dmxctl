import { useCallback, useEffect, useRef, useState } from 'react'
import { ApiError, Command, PanelState, PinError, fetchState, sendCommand } from './api'

const POLL_MS = 1000

/** 様子を1秒ごとに読み直し、操作を送る。操作の応答にも最新の様子が入っている */
export function usePanel() {
  const [state, setState] = useState<PanelState | null>(null)
  // 接続できているか。操作が断られただけでは変えない
  const [offline, setOffline] = useState(false)
  // 操作が断られたときの理由。次に成功するまで出す
  const [notice, setNotice] = useState('')
  const [needPin, setNeedPin] = useState(false)
  const busy = useRef(false)

  const ok = useCallback((s: PanelState) => {
    setState(s)
    setOffline(false)
    setNotice('')
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

  const poll = useCallback(async () => {
    if (busy.current) return
    busy.current = true
    try {
      ok(await fetchState())
    } catch (e) {
      fail(e)
    } finally {
      busy.current = false
    }
  }, [ok, fail])

  const send = useCallback(
    async (c: Command) => {
      try {
        ok(await sendCommand(c))
      } catch (e) {
        fail(e)
      }
    },
    [ok, fail],
  )

  useEffect(() => {
    poll()
    const t = setInterval(poll, POLL_MS)
    return () => clearInterval(t)
  }, [poll])

  return { state, offline, notice, needPin, send, poll }
}
