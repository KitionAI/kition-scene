import { useMemo, useState } from 'react'
import { Dialog, useKitEditor, type KitCommand } from '@kition/sdk/editor'

/** One thing the search finds: a command, where its menu row is, and its key. */
export type SearchEntry = { id: string; label: string; path: string; shortcut: string | null; run: () => void }

/** Commands a person reaches (a menu row or a key) and can run now, as search entries. */
export function searchEntries(commands: readonly KitCommand[], enabled: (id: string) => boolean, phrase: (english: string) => string, run: (id: string) => void): SearchEntry[] {
  return commands
    .filter((command) => (command.menu?.length || command.shortcut) && enabled(command.id))
    .map((command) => ({
      id: command.id,
      label: phrase(command.title),
      path: (command.menu ?? []).map((part) => phrase(part)).join(' ▸ '),
      shortcut: command.shortcut ?? null,
      run: () => run(command.id),
    }))
}

/** Whether `entry` matches every word of `query` (its name, its menu path or its id). */
function matches(entry: SearchEntry, query: string) {
  const text = `${entry.label} ${entry.path} ${entry.id}`.toLowerCase()
  return query
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .every((word) => text.includes(word))
}

/**
 * F3's search: a field and the commands it finds in the current mode (name, menu path, key); the
 * arrow keys move, Enter runs the lit one, Escape closes.
 */
export function CommandSearch({ entries, onClose }: { entries: SearchEntry[]; onClose: () => void }) {
  const { t } = useKitEditor()
  const [query, setQuery] = useState('')
  const [lit, setLit] = useState(0)
  const found = useMemo(() => entries.filter((entry) => matches(entry, query)).slice(0, 60), [entries, query])
  const chosen = found[Math.min(lit, found.length - 1)]
  const choose = (entry: SearchEntry | undefined) => {
    if (!entry) return
    onClose()
    entry.run()
  }
  return (
    <Dialog title={t('searchTitle')} testId="scene-search" onClose={onClose} actions={null}>
      <input
        className="kui-field scene-search-field"
        aria-label={t('searchTitle')}
        placeholder={t('searchPlaceholder')}
        value={query}
        autoFocus
        onChange={(event) => {
          setQuery(event.target.value)
          setLit(0)
        }}
        onKeyDown={(event) => {
          if (event.key === 'ArrowDown') setLit((index) => Math.min(index + 1, Math.max(0, found.length - 1)))
          else if (event.key === 'ArrowUp') setLit((index) => Math.max(0, index - 1))
          else if (event.key === 'Enter') choose(chosen)
          else return
          event.preventDefault()
          event.stopPropagation()
        }}
        data-testid="scene-search-field"
      />
      <ul className="scene-search-list" role="listbox" aria-label={t('searchTitle')}>
        {found.map((entry, index) => (
          <li
            key={entry.id}
            role="option"
            aria-selected={entry === chosen}
            className="scene-search-item"
            onPointerMove={() => lit !== index && setLit(index)}
            onClick={() => choose(entry)}
            data-testid="scene-search-item"
            data-id={entry.id}
          >
            <span className="scene-search-label">{entry.label}</span>
            <span className="scene-search-path">{entry.path}</span>
            {entry.shortcut ? <kbd>{entry.shortcut}</kbd> : null}
          </li>
        ))}
        {found.length ? null : <li className="kui-muted scene-search-none">{t('searchNothing')}</li>}
      </ul>
    </Dialog>
  )
}
