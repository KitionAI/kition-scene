/** The development host (dev.html): opens `public/sample/Sample.kscene`, or a file chosen in the page. */
import { startDevHost } from '@kition/sdk/dev-host'

// Presence is served, so the editor shares its selection and pointer and shows the people the page
// plays; view state too, so the person's view, selection and frame come back after a reload.
startDevHost({ sample: './sample/Sample.kscene', presence: true, viewState: true })

// The page gives the editor the keyboard once it loads, as a desktop host's window would: its
// shortcuts (A, Home, Tab) answer before any click. A choice the person made since (the file
// picker) keeps the focus.
const frame = document.getElementById('kit')
if (frame instanceof HTMLIFrameElement) {
  frame.addEventListener('load', () => {
    if (frame.src === 'about:blank') return
    if (document.activeElement === document.body || document.activeElement == null) frame.focus()
  })
}
