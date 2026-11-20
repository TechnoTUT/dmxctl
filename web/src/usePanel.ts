import { useCallback, useEffect, useRef, useState } from 'react'
import { Command, PanelState, PinError, fetchState, sendCommand } from './api'

const POLL_MS = 1000

/** 様子を1秒ごとに読み直し、操作を送る。操作の応答にも最新の様子が入っている */
export function usePanel() {
  const [state, setState] = useState<PanelState | null>(null)
  const [error, setError] = useState('')
  const [needPin, setNeedPin] = useState(false)
  const busy = useRef(false)

  const fail = useCallback((e: unknown, offline: string) => {
    if (e instanceof PinError) {
      setNeedPin(true)
      return
    }
    setError(offline || (e instanceof Error ? e.message : String(e)))
  }, [])

  const poll = useCallback(async () => {
    if (busy.current) return
    busy.current = true
    try {
      setState(await fetchState())
      setError('')
      setNeedPin(false)
    } catch (e) {
      fail(e, 'つながりません')
    } finally {
      busy.current = false
    }
  }, [fail])

  const send = useCallback(
    async (c: Command) => {
      try {
        setState(await sendCommand(c))
        setError('')
        setNeedPin(false)
      } catch (e) {
        fail(e, '')
      }
    },
    [fail],
  )

  useEffect(() => {
    poll()
    const t = setInterval(poll, POLL_MS)
    return () => clearInterval(t)
  }, [poll])

  return { state, error, needPin, send, poll }
}
