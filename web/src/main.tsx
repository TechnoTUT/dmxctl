import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { App } from './App'
import favicon from './assets/favicon.svg'
import './styles.css'

// 配信できるのは1枚のHTMLだけなので、ファビコンはデータURIとして埋め込む
const icon = document.createElement('link')
icon.rel = 'icon'
icon.href = favicon
document.head.appendChild(icon)

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
