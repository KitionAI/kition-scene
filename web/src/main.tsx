import { createRoot } from 'react-dom/client'
import { connectHost, createKitStrings, withPhrases } from '@kition/kit-editor'
import '@kition/kit-editor/styles.css'
import './app.css'
import { App } from './App'
import { CATALOGS, PHRASES } from './strings'
import manifest from '../../kit.json'
import './commandIcons'

const root = createRoot(document.getElementById('root')!)

// Opened outside a host, the page says where it belongs instead of waiting forever.
const alone = window.setTimeout(() => {
  const t = createKitStrings(CATALOGS, navigator.language)
  root.render(<div className="kui-alone">{t('noHost')}</div>)
}, 3000)

void connectHost({ id: 'kit.scene', version: '0.1.4', capabilities: ['ai', 'replica', 'presence'], brand: manifest.ui.brand, display: manifest.ui.display as 'menus' }).then((host) => {
  window.clearTimeout(alone)
  // The kit's own Chinese names for the engine's English labels.
  const phrase = withPhrases((english) => english, PHRASES, host.init.locale)
  // No StrictMode: its doubled effects would open the document twice, in two engine workers.
  root.render(<App host={host} phrase={phrase} />)
})
