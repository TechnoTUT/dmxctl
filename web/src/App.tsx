import { Moon, Sun } from 'lucide-react'
import { useState } from 'react'
import { savePin } from './api'
import { Auto, Colors, Master, PinBox, Scenes, Status } from './components'
import logo from './assets/logo.svg'
import logoDark from './assets/logo_dark.svg'
import { usePanel } from './usePanel'

function useTheme() {
  const [dark, setDark] = useState(() => document.documentElement.classList.contains('dark'))
  const toggle = () => {
    const next = !dark
    document.documentElement.classList.toggle('dark', next)
    try {
      localStorage.setItem('theme', next ? 'dark' : 'light')
    } catch {
      // 保存できなくても、そのページの間は切り替わる
    }
    setDark(next)
  }
  return { dark, toggle }
}

export function App() {
  const { state, error, needPin, send, poll } = usePanel()
  const { dark, toggle } = useTheme()
  const online = !error && state !== null
  return (
    <>
      <header className="sticky top-0 z-30 flex items-center justify-between gap-3 border-b border-slate-200 bg-white/90 px-4 py-3 shadow-sm backdrop-blur dark:border-slate-800 dark:bg-slate-900/80">
        <div className="flex min-w-0 items-center gap-3">
          <img src={logo} alt="TechnoTUT" className="h-9 w-auto max-w-[140px] object-contain sm:h-10 sm:max-w-[170px] dark:hidden" />
          <img src={logoDark} alt="TechnoTUT" className="hidden h-9 w-auto max-w-[140px] object-contain sm:h-10 sm:max-w-[170px] dark:block" />
          <div className="h-8 w-px shrink-0 bg-slate-200 dark:bg-slate-800" />
          <div className="min-w-0">
            <h1 className="truncate text-sm font-bold tracking-tight">DMX 操作パネル</h1>
            <p className="truncate text-[11px] text-slate-500 dark:text-slate-400">{error ? `⚠ ${error}` : 'dmxctl'}</p>
          </div>
        </div>
        <div className="flex items-center gap-2">
          <div className={`flex items-center gap-1.5 rounded-xl border px-3 py-1.5 text-xs font-semibold ${online
            ? 'border-emerald-200 bg-emerald-50 text-emerald-600 dark:border-emerald-900 dark:bg-emerald-950/40 dark:text-emerald-400'
            : 'border-rose-200 bg-rose-50 text-rose-600 dark:border-rose-900 dark:bg-rose-950/40 dark:text-rose-400'}`}>
            <span className={`size-2 rounded-full ${online ? 'animate-pulse bg-emerald-500' : 'bg-rose-500'}`} />
            <span className="hidden sm:inline">{online ? 'Online' : 'Offline'}</span>
          </div>
          <button
            onClick={toggle}
            title="テーマを切り替える"
            className="rounded-xl border border-slate-200 p-2 text-slate-500 transition hover:bg-slate-100 dark:border-slate-800 dark:text-slate-400 dark:hover:bg-slate-800"
          >
            {dark ? <Sun className="size-5 text-amber-400" /> : <Moon className="size-5 text-slate-600" />}
          </button>
        </div>
      </header>
      {/* 広い画面では2列にして、スクロールせずに全体を操作できるようにする */}
      <main className="mx-auto max-w-3xl px-4 pb-10 pt-4 lg:flex lg:h-[calc(100dvh-4.25rem)] lg:max-w-[1600px] lg:flex-col lg:pb-3">
        {needPin && <PinBox onSubmit={(p) => { savePin(p); poll() }} />}
        {state && (
          <div className="lg:grid lg:min-h-0 lg:flex-1 lg:grid-cols-2 lg:gap-x-4">
            <div className="lg:min-h-0 lg:overflow-y-auto">
              <Status s={state} />
              <Master s={state} send={send} />
              <Auto s={state} send={send} />
            </div>
            <div className="lg:min-h-0 lg:overflow-y-auto">
              <Colors s={state} send={send} />
              <Scenes s={state} send={send} />
            </div>
          </div>
        )}
        <p className="text-xs text-slate-500 dark:text-slate-400 lg:shrink-0">このページは dmxctl が出しています。1秒ごとに様子を読み直します。</p>
      </main>
    </>
  )
}
