import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import '@fontsource/funnel-display/400.css'
import '@fontsource/funnel-display/600.css'
import '@fontsource/funnel-sans/400.css'
import '@fontsource/funnel-sans/500.css'
import '@fontsource/funnel-sans/600.css'
import './index.css'
import App from './v2/App'

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
