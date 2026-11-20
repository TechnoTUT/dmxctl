import { savePin } from './api'
import { Auto, Colors, Master, PinBox, Scenes, Status } from './components'
import { usePanel } from './usePanel'

export function App() {
  const { state, error, needPin, send, poll } = usePanel()
  return (
    <>
      <h1>💡 DMX 操作パネル <span className="small">{error && `⚠ ${error}`}</span></h1>
      {needPin && <PinBox onSubmit={(p) => { savePin(p); poll() }} />}
      {state && (
        <>
          <Status s={state} />
          <Master s={state} send={send} />
          <Auto s={state} send={send} />
          <Colors s={state} send={send} />
          <Scenes s={state} send={send} />
        </>
      )}
      <p className="small">このページは dmxctl が出しています。1秒ごとに様子を読み直します。</p>
    </>
  )
}
