/** The development host (dev.html): opens `public/sample/Sample.kscene`, or a file chosen in the page. */
import { startDevHost } from '@kition/kit-dev-host'

// Presence is served, so the editor shares its selection and pointer and shows the people the page plays.
startDevHost({ sample: './sample/Sample.kscene', presence: true })
