/** The development host (dev.html): opens `public/sample/Sample.kscene`, or a file chosen in the page. */
import { startDevHost } from '@kition/sdk/dev-host'

// Presence is served, so the editor shares its selection and pointer and shows the people the page
// plays; view state too, so the person's view, selection and frame come back after a reload.
startDevHost({ sample: './sample/Sample.kscene', presence: true, viewState: true })
