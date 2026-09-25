<script lang="ts">
  // Read from the bundle rather than typed in, so the About line cannot
  // fall behind the version the installer actually carries.
  const APP_VERSION = __APP_VERSION__
  // OWNER: worker W6. The settings window — every control patches a real
  // settings field; nothing here is decorative.

  import { open, save } from '@tauri-apps/plugin-dialog'

  import {
    clearHistory,
    exportData,
    getClipboardHistoryEnabled,
    getStorageStats,
    importData,
    onItemAdded,
    onItemsDeleted,
    onStorageWarning,
    onStoreProgress,
    relocateStore,
    runCleanupNow,
    setCaptureEnabled,
    setClipboardHistoryEnabled,
    type UnlistenFn,
  } from '../lib/ipc'
  import { settings } from '../lib/stores/settings.svelte'
  import { t } from '../lib/i18n/index.svelte'
  import LanguagePicker from '../lib/components/LanguagePicker.svelte'
  import NumberField from '../lib/components/NumberField.svelte'
  import ThemePreview from '../lib/components/ThemePreview.svelte'
  import { THEMES, type TimeFormat } from '../lib/types'
  import type {
    CleanupResult,
    ImportMode,
    SettingsPatch,
    StorageStats,
    StoreProgress,
  } from '../lib/types'

  const MB = 1024 * 1024

  type SectionId = 'general' | 'storage' | 'appearance' | 'privacy' | 'data' | 'about'

  const SECTIONS: Array<{ id: SectionId; labelKey: string }> = [
    { id: 'general', labelKey: 'settings.tab.general' },
    { id: 'storage', labelKey: 'settings.tab.storage' },
    { id: 'appearance', labelKey: 'settings.tab.appearance' },
    { id: 'privacy', labelKey: 'settings.tab.privacy' },
    { id: 'data', labelKey: 'settings.tab.data' },
    { id: 'about', labelKey: 'settings.tab.about' },
  ]

  const MODIFIER_KEYS = new Set(['Control', 'Alt', 'Shift', 'Meta', 'CapsLock', 'NumLock', 'ScrollLock'])

  // Keys, not resolved strings: this table is built once, when the window is
  // created and before the language is known, so it stores what to look up
  // rather than what it looked up.
  const WIN_RESERVED_KEYS: Record<string, string> = {
    v: 'hotkey.winV',
    e: 'hotkey.winE',
    r: 'hotkey.winR',
    l: 'hotkey.winL',
    d: 'hotkey.winD',
    i: 'hotkey.winI',
    s: 'hotkey.winS',
    k: 'hotkey.winK',
    g: 'hotkey.winG',
    t: 'hotkey.winT',
    x: 'hotkey.winX',
    u: 'hotkey.winU',
    w: 'hotkey.winW',
    a: 'hotkey.winA',
    h: 'hotkey.winH',
    f: 'hotkey.winF',
    m: 'hotkey.winM',
    z: 'hotkey.winZ',
    p: 'hotkey.winP',
    y: 'hotkey.winY',
  }

  const KIND_COLORS: Record<string, string> = {
    image: '#7aa2ff',
    video: '#bb9af7',
    text: '#9ece6a',
    file: '#e0af68',
    other: '#565f89',
  }

  let section = $state<SectionId>('general')
  let stats = $state<StorageStats | null>(null)
  let progress = $state<StoreProgress | null>(null)
  let error = $state<string | null>(null)
  let cleanup = $state<CleanupResult | null>(null)
  let cleanDays = $state(30)
  let recording = $state(false)
  let hotkeyError = $state<string | null>(null)
  let hotkeyWarning = $state<string | null>(null)
  let newBlocked = $state('')
  let importMode = $state<ImportMode>('merge')
  let confirmClear = $state(false)
  let confirmReset = $state(false)
  // Live Windows clipboard-history state (Win+V). null = not yet read.
  let clipboardHistory = $state<boolean | null>(null)

  const usageSegments = $derived.by(() => {
    const s = stats
    if (!s || s.totalBytes <= 0 || s.byKind.length === 0) return null
    return s.byKind.map((k) => ({ kind: k.kind, bytes: k.bytes, pct: (k.bytes / s.totalBytes) * 100 }))
  })

  $effect(() => {
    void settings.init()
    void refreshStats()
    let unlisten: (() => void) | null = null
    void onStoreProgress((p) => {
      if (p.phase === 'completed') {
        // The banner is for an operation in progress; a permanent "completed"
        // strip would sit over the UI for the life of the window.
        progress = null
        return
      }
      progress = p
    }).then((fn) => {
      unlisten = fn
    })
    // The meter is a snapshot of the DB; keep it honest while this window is
    // open by refreshing on every event that changes the store. The janitor's
    // prunes arrive as storage-warning (it does not always emit items-deleted).
    let statListeners: UnlistenFn[] = []
    void onItemAdded(() => refreshStats()).then((fn) => statListeners.push(fn))
    void onItemsDeleted(() => refreshStats()).then((fn) => statListeners.push(fn))
    void onStorageWarning(() => refreshStats()).then((fn) => statListeners.push(fn))
    // Windows clipboard history is OS state the user can change in Windows
    // Settings; re-read it every time this window is (re)shown so the toggle
    // reflects reality rather than a stale first read.
    refreshClipboardHistory()
    const onWindowVisible = (): void => {
      if (document.visibilityState === 'visible') refreshClipboardHistory()
    }
    document.addEventListener('visibilitychange', onWindowVisible)
    window.addEventListener('focus', onWindowVisible)
    return () => {
      unlisten?.()
      for (const fn of statListeners) fn()
      document.removeEventListener('visibilitychange', onWindowVisible)
      window.removeEventListener('focus', onWindowVisible)
    }
  })

  // A capture or prune while the window sat on another section should not
  // leave the panel lying when the user finally navigates to it.
  $effect(() => {
    if (section === 'storage') void refreshStats()
  })

  // Landing on General re-reads the live system state too, so the toggle is
  // never an echo of a value the user changed elsewhere.
  $effect(() => {
    if (section === 'general') refreshClipboardHistory()
  })

  $effect(() => {
    if (!recording) return
    const onKey = (e: KeyboardEvent): void => {
      e.preventDefault()
      e.stopPropagation()
      if (e.key === 'Escape') {
        recording = false
        return
      }
      if (MODIFIER_KEYS.has(e.key)) return
      const chord = formatChord(e)
      if (chord === null) {
        hotkeyError = t('hotkey.errInvalidKey')
        return
      }
      const reserved = checkReserved(e)
      if (reserved !== null) {
        hotkeyError = reserved
        return
      }
      hotkeyError = null
      hotkeyWarning = advisoryWarnings(e, chord)
      recording = false
      patch({ hotkey: { binding: chord } })
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  })

  function patch(p: SettingsPatch): void {
    void settings.patch(p).catch((err) => {
      error = String(err)
    })
  }

  function refreshStats(): Promise<void> {
    return getStorageStats()
      .then((s) => {
        stats = s
      })
      .catch(() => {
        // Backend not reachable yet (parallel build); retry on next action.
      })
  }

  /** Reads the live Windows clipboard-history state; null stays until the
   * first successful read so the toggle never flips from an unknown state. */
  function refreshClipboardHistory(): void {
    getClipboardHistoryEnabled()
      .then((enabled) => {
        clipboardHistory = enabled
      })
      .catch((err) => {
        // Keep the last known value; a failed read is not a reason to claim a
        // state. Surface it so the user knows the toggle may be stale.
        error = `Could not read Windows clipboard history: ${String(err)}`
      })
  }

  /** Writes the toggle to Windows and only shows it as toggled once the write
   * succeeded; a failed write reverts the control and explains itself. */
  function onClipboardHistoryChange(checked: boolean): void {
    const before = clipboardHistory
    clipboardHistory = checked
    setClipboardHistoryEnabled(checked)
      .then(() => {
        // The write is explicit; show what we wrote. The next window-show
        // re-reads and would correct any drift.
        clipboardHistory = checked
      })
      .catch((err) => {
        clipboardHistory = before
        error = `Could not change Windows clipboard history: ${String(err)}`
      })
  }

  function fmtBytes(n: number): string {
    if (n < 1024) return `${n} B`
    const units = ['KB', 'MB', 'GB', 'TB']
    let v = n
    let u = -1
    do {
      v /= 1024
      u++
    } while (v >= 1024 && u < units.length - 1)
    return `${v.toFixed(v >= 100 ? 0 : v >= 10 ? 1 : 2)} ${units[u]}`
  }

  function kindColor(k: string): string {
    return KIND_COLORS[k] ?? KIND_COLORS.other!
  }

  /** The backend reports kinds as `image`, `text`, `link`, `file`, `video` and
   * `other`; the storage legend shows them to the user, so they get names. */
  function kindName(k: string): string {
    return t(`kind.${k}`)
  }

  // -- hotkey recording --------------------------------------------------------

  function formatChord(e: KeyboardEvent): string | null {
    let key = e.key
    if (/^[a-z]$/i.test(key)) key = key.toUpperCase()
    else if (/^[0-9]$/.test(key)) {
      // keep
    } else if (/^F([1-9]|1[0-9]|2[0-4])$/.test(key)) {
      // keep
    } else {
      return null
    }
    const parts: string[] = []
    if (e.ctrlKey) parts.push('Ctrl')
    if (e.altKey) parts.push('Alt')
    if (e.shiftKey) parts.push('Shift')
    if (e.metaKey) parts.push('Win')
    parts.push(key)
    return parts.join('+')
  }

  function checkReserved(e: KeyboardEvent): string | null {
    if (!e.metaKey) return null
    const key = WIN_RESERVED_KEYS[e.key.toLowerCase()]
    if (key) {
      const name = t(key)
      return t('hotkey.reservedChord', { name })
    }
    return null
  }

  function advisoryWarnings(e: KeyboardEvent, chord: string): string | null {
    const warns: string[] = []
    if (!e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey) {
      warns.push(t('hotkey.warnBareKey'))
    }
    if (e.ctrlKey && !e.metaKey && ['c', 'x', 'v'].includes(e.key.toLowerCase())) {
      warns.push(t('hotkey.warnSystemCopy'))
    }
    if (e.metaKey) {
      warns.push(t('hotkey.warnWinChord'))
    }
    if (chord === 'Alt+V') {
      warns.push(t('hotkey.infoDefault'))
    }
    return warns.length > 0 ? warns.join(' ') : null
  }

  // -- storage ------------------------------------------------------------------

  async function pickStoreLocation(): Promise<void> {
    const dir = await open({ directory: true, title: t('dialog.chooseStore') })
    if (!dir) return
    try {
      await relocateStore(dir)
      await settings.reload()
      await refreshStats()
    } catch (err) {
      error = `Could not move the store: ${String(err)}`
    } finally {
      // The Rust side emits store-progress with a trailing "completed" phase;
      // a failure may not, so never leave the banner stuck on an error path.
      progress = null
    }
  }

  async function runClean(): Promise<void> {
    try {
      cleanup = await runCleanupNow(cleanDays)
      await refreshStats()
    } catch (err) {
      error = `Clean up failed: ${String(err)}`
    }
  }

  function onCaptureEnabled(v: boolean): void {
    patch({ behavior: { captureEnabled: v } })
    void setCaptureEnabled(v).catch((err) => {
      error = String(err)
    })
  }

  // -- privacy ------------------------------------------------------------------

  function addBlocked(): void {
    const p = newBlocked.trim().toLowerCase()
    newBlocked = ''
    if (!p) return
    const list = [...settings.current.privacy.blockedProcesses]
    if (list.includes(p)) return
    list.push(p)
    patch({ privacy: { blockedProcesses: list } })
  }

  function removeBlocked(p: string): void {
    patch({
      privacy: {
        blockedProcesses: settings.current.privacy.blockedProcesses.filter((x) => x !== p),
      },
    })
  }

  // -- data ---------------------------------------------------------------------

  async function doExport(): Promise<void> {
    const target = await save({
      title: t('dialog.exportTitle'),
      defaultPath: 'rebuffer-export.rbx',
      filters: [{ name: t('dialog.archiveFilter'), extensions: ['rbx'] }],
    })
    if (!target) return
    try {
      await exportData(target)
    } catch (err) {
      error = `Export failed: ${String(err)}`
    } finally {
      progress = null
    }
  }

  async function doImport(): Promise<void> {
    const picked = await open({
      title: t('dialog.importTitle'),
      multiple: false,
      filters: [{ name: 'Rebuffer archive', extensions: ['rbx'] }],
    })
    const path = typeof picked === 'string' ? picked : picked?.[0]
    if (!path) return
    try {
      await importData(path, importMode)
      await settings.reload()
      await refreshStats()
    } catch (err) {
      error = `Import failed: ${String(err)}`
    } finally {
      progress = null
    }
  }

  function clearHistoryNow(): void {
  if (!confirmClear) {
    confirmClear = true
    confirmReset = false
    return
  }
  confirmClear = false
  void clearHistory(false)
    .then((r) => {
      cleanup = r
      void refreshStats()
    })
    .catch((err) => {
      error = String(err)
    })
}

function resetEverything(): void {
  if (!confirmReset) {
    confirmReset = true
    confirmClear = false
    return
  }
  confirmReset = false
  void clearHistory(true)
    .then((r) => {
      cleanup = r
      void refreshStats()
    })
    .catch((err) => {
      error = String(err)
    })
}
  /// What --accent resolves to right now: the override when set, otherwise
  /// whatever the active theme defines. Read from the document so the picker
  /// shows the real colour rather than an empty value.
  const effectiveAccent = $derived.by(() => {
    const custom = settings.current.appearance.accent
    if (custom) return custom
    void settings.current.appearance.theme // re-read when the theme changes
    const v = getComputedStyle(document.documentElement).getPropertyValue('--accent').trim()
    return v || '#7aa2ff'
  })

</script>

<div class="settings">
  {#if progress}
    <div class="progress">
      <div class="progress-track">
        <div
          class="progress-fill"
          style="width:{progress.total > 0 ? Math.min(100, (progress.done / progress.total) * 100) : 0}%"
        ></div>
      </div>
      <span>{progress.phase}{progress.total > 0 ? ` — ${progress.done} / ${progress.total}` : ''}</span>
    </div>
  {/if}

  {#if error}
    <div class="error-banner" role="alert">
      <span>{error}</span>
      <button onclick={() => { error = null }}>{t('settings.dismiss')}</button>
    </div>
  {/if}

  <nav>
    {#each SECTIONS as s}
      <button class:active={section === s.id} onclick={() => { section = s.id }}>
        {t(s.labelKey)}
      </button>
    {/each}
  </nav>

  <main class="content">
    {#if section === 'general'}
      <section>
        <h2>{t('settings.tab.general')}</h2>

        <div class="field">
          <span class="field-label">{t('settings.hotkey')}</span>
          <div class="row">
            <button class="hotkey-box" onclick={() => { recording = true }} title={t('settings.hotkeyHint')} aria-label={t('settings.hotkeyAria')}>
              {recording ? t('settings.pressChord') : settings.current.hotkey.binding}
            </button>
            {#if !recording}
              <button onclick={() => { recording = true }}>{t('settings.hotkeyRecord')}</button>
            {/if}
          </div>
          {#if recording}
            <p class="hint">{t('settings.hotkeyRecording')}</p>
          {/if}
          {#if hotkeyError}
            <p class="error">{hotkeyError}</p>
          {/if}
          {#if hotkeyWarning && !recording}
            <p class="hint">{hotkeyWarning}</p>
          {/if}
        </div>

        <label class="toggle">
          <input
            type="checkbox"
            checked={settings.current.hotkey.aggressiveMode}
            onchange={(e) => patch({ hotkey: { aggressiveMode: e.currentTarget.checked } })}
          />
          <span>{t('settings.aggressiveMode')}</span>
        </label>
        {#if settings.current.hotkey.aggressiveMode}
          <p class="hint">
            {t('settings.aggressiveModeHint')}
          </p>
        {/if}

        <div class="field">
          <label class="toggle">
            <input
              type="checkbox"
              checked={clipboardHistory ?? false}
              disabled={clipboardHistory === null}
              onchange={(e) => onClipboardHistoryChange(e.currentTarget.checked)}
            />
            <span>{t('settings.winV')}</span>
          </label>
          <p class="hint">
            {t('settings.winVHint')}
          </p>
        </div>

        <label class="toggle">
          <input
            type="checkbox"
            checked={settings.current.behavior.launchOnStartup}
            onchange={(e) => patch({ behavior: { launchOnStartup: e.currentTarget.checked } })}
          />
          <span>{t('settings.launchOnStartup')}</span>
        </label>

        <label class="toggle">
          <input
            type="checkbox"
            checked={settings.current.behavior.silentStart}
            onchange={(e) => patch({ behavior: { silentStart: e.currentTarget.checked } })}
          />
          <span>{t('settings.silentStart')}</span>
        </label>

        <label class="toggle">
          <input
            type="checkbox"
            checked={settings.current.behavior.captureEnabled}
            onchange={(e) => onCaptureEnabled(e.currentTarget.checked)}
          />
          <span>{t('settings.captureEnabled')}</span>
        </label>
        {#if !settings.current.behavior.captureEnabled}
          <p class="hint">{t('settings.capturePaused')}</p>
        {/if}

        <label class="toggle">
          <input
            type="checkbox"
            checked={settings.current.behavior.autoPaste}
            onchange={(e) => patch({ behavior: { autoPaste: e.currentTarget.checked } })}
          />
          <span>{t('settings.autoPaste')}</span>
        </label>
        {#if settings.current.behavior.autoPaste}
          <p class="hint">
            {t('settings.autoPasteHint')}
          </p>
        {/if}

        <label class="toggle">
          <input
            type="checkbox"
            checked={settings.current.behavior.pasteAsPlainText}
            onchange={(e) => patch({ behavior: { pasteAsPlainText: e.currentTarget.checked } })}
          />
          <span>{t('settings.pastePlain')}</span>
        </label>

        <label class="toggle">
          <input
            type="checkbox"
            checked={settings.current.behavior.closeOnCopy}
            onchange={(e) => patch({ behavior: { closeOnCopy: e.currentTarget.checked } })}
          />
          <span>{t('settings.closeOnCopy')}</span>
        </label>
      </section>

    {:else if section === 'storage'}
      <section>
        <h2>{t('settings.tab.storage')}</h2>

        <div class="field">
          <span class="field-label">{t('settings.storeLocation')}</span>
          <div class="row">
            <code class="path">{settings.current.storage.path || t('settings.storeDefault')}</code>
            <button onclick={() => void pickStoreLocation()} aria-label={t('settings.changeStoreAria')}>{t('settings.change')}</button>
          </div>
          <p class="hint">{t('settings.storeLocationHint')}</p>
        </div>

        <div class="field">
          <label class="field-label">
            {t('settings.retention')}
            <span class="row">
              <NumberField
                min={1}
                max={30}
                width="92px"
                value={settings.current.storage.retentionDays}
                onchange={(n) => patch({ storage: { retentionDays: n ?? 1 } })}
              />
              <span>{t('settings.days')}</span>
            </span>
          </label>
        </div>

        <div class="field">
          <label class="field-label">
            {t('settings.tempFiles')}
            <span class="row">
              <NumberField
                min={1}
                max={90}
                width="92px"
                value={settings.current.storage.tempFilesDays}
                onchange={(n) => patch({ storage: { tempFilesDays: n ?? 1 } })}
              />
              <span>{t('settings.days')}</span>
            </span>
          </label>
          <p class="hint">
            {t('settings.tempFilesHint')}
          </p>
        </div>

        <div class="field">
          <label class="field-label">
            {t('settings.maxItemSize')}
            <span class="row">
              <NumberField
                min={1}
                width="100px"
                value={Math.round(settings.current.storage.maxItemBytes / MB)}
                onchange={(n) => patch({ storage: { maxItemBytes: Math.max(1, n ?? 1) * MB } })}
              />
              <span>{t('settings.mb')}</span>
            </span>
          </label>
          <p class="hint">{t('settings.maxItemSizeHint')}</p>
        </div>

        <div class="field">
          <label class="field-label">
            {t('settings.storeCap')}
            <span class="row">
              <NumberField
                min={1}
                width="100px"
                allowEmpty
                value={settings.current.storage.maxStoreBytes === null
                  ? null
                  : Math.round(settings.current.storage.maxStoreBytes / MB)}
                onchange={(n) =>
                  patch({ storage: { maxStoreBytes: n === null ? null : Math.max(1, n) * MB } })}
              />
              <span>{t('settings.storeCapUnit')}</span>
            </span>
          </label>
        </div>

        <label class="toggle">
          <input
            type="checkbox"
            checked={settings.current.storage.notifyWhenFull}
            onchange={(e) => patch({ storage: { notifyWhenFull: e.currentTarget.checked } })}
          />
          <span>{t('settings.notifyFull')}</span>
        </label>

        <div class="usage">
          <div class="usage-head">
            <span>{t('settings.storageUsage')}</span>
            <span>
              {t('settings.usageSummary', {
                size: fmtBytes(stats?.totalBytes ?? 0),
                count: stats?.totalItems ?? 0,
              })}
            </span>
          </div>
          <div class="meter">
            {#each usageSegments ?? [] as seg}
              <div
                class="meter-seg"
                style="width:{seg.pct}%;background:{kindColor(seg.kind)}"
                title="{kindName(seg.kind)} — {fmtBytes(seg.bytes)}"
              ></div>
            {/each}
          </div>
          <ul class="legend">
            {#each stats?.byKind ?? [] as k}
              <li>
                <span class="dot" style="background:{kindColor(k.kind)}"></span>
                {t('settings.usageKind', {
                  kind: kindName(k.kind),
                  count: k.count,
                  size: fmtBytes(k.bytes),
                })}
              </li>
            {/each}
          </ul>
          <div class="row">
            <NumberField
              min={0}
              max={3650}
              width="92px"
              value={cleanDays}
              onchange={(n) => { cleanDays = n ?? 0 }}
            />
            <span>{t('settings.cleanOlder')}</span>
            <button onclick={() => void runClean()}>{t('settings.cleanNow')}</button>
          </div>
          {#if cleanup}
            <p class="ok">
              {t('settings.cleanupDone', {
                count: cleanup.removedItems,
                size: fmtBytes(cleanup.freedBytes),
              })}
            </p>
          {/if}
        </div>
      </section>

    {:else if section === 'appearance'}
      <section>
        <h2>{t('settings.tab.appearance')}</h2>

        <div class="field">
          <span class="field-label">{t('settings.theme')}</span>
          <div class="theme-strip" role="group" aria-label={t('settings.theme')}>
            {#each THEMES as themeName}
              <ThemePreview
                theme={themeName}
                selected={settings.current.appearance.theme === themeName}
                onselect={(v) => patch({ appearance: { theme: v } })}
              />
            {/each}
          </div>
          <p class="hint">{t('settings.themeHint')}</p>
        </div>

        <div class="field">
          <span class="field-label">{t('settings.language')}</span>
          <LanguagePicker
            value={settings.current.appearance.language}
            onselect={(v) => patch({ appearance: { language: v } })}
          />
          <p class="hint">{t('settings.languageHint')}</p>
        </div>

        <div class="field">
          <span class="field-label">{t('settings.popupSize')}</span>
          <div class="radios">
            <label>
              <input
                type="radio"
                name="sizeMode"
                checked={settings.current.window.sizeMode === 'percent'}
                onchange={() => patch({ window: { sizeMode: 'percent' } })}
              />
              {t('settings.percentOfMonitor')}
            </label>
            <label>
              <input
                type="radio"
                name="sizeMode"
                checked={settings.current.window.sizeMode === 'fixed'}
                onchange={() => patch({ window: { sizeMode: 'fixed' } })}
              />
              {t('settings.fixedSize')}
            </label>
          </div>
        </div>

        {#if settings.current.window.sizeMode === 'percent'}
          <div class="field">
            <label class="field-label">
              {t('settings.percentOfMonitor')}
              <NumberField
                min={10}
                max={100}
                width="92px"
                value={settings.current.window.percentOfMonitor}
                onchange={(n) => patch({ window: { percentOfMonitor: n ?? 10 } })}
              />
            </label>
          </div>
        {:else}
          <div class="field">
            <span class="field-label">{t('settings.fixedWidthHeight')}</span>
            <div class="row">
              <NumberField
                min={320}
                max={3840}
                step={10}
                width="104px"
                value={settings.current.window.fixed.width}
                ariaLabel={t('settings.fixedWidth')}
                onchange={(n) =>
                  patch({
                    window: {
                      fixed: {
                        width: n ?? 320,
                        height: settings.current.window.fixed.height,
                      },
                    },
                  })}
              />
              <span>×</span>
              <NumberField
                min={320}
                max={2160}
                step={10}
                width="104px"
                value={settings.current.window.fixed.height}
                ariaLabel={t('settings.fixedHeight')}
                onchange={(n) =>
                  patch({
                    window: {
                      fixed: {
                        width: settings.current.window.fixed.width,
                        height: n ?? 320,
                      },
                    },
                  })}
              />
            </div>
          </div>
        {/if}

        <div class="field">
          <label class="toggle">
            <input
              type="checkbox"
              checked={settings.current.window.dragBar}
              onchange={(e) => patch({ window: { dragBar: e.currentTarget.checked } })}
            />
            <span>{t('settings.dragBar')}</span>
          </label>
          <p class="hint">
            {t('settings.dragBarHint')}
          </p>
        </div>

        <div class="field">
          <label class="field-label">
            {t('settings.gridZoom')}
            <NumberField
              min={1}
              max={5}
              width="80px"
              value={settings.current.window.zoomStep}
              onchange={(n) => patch({ window: { zoomStep: n ?? 1 } })}
            />
          </label>
        </div>

        <label class="toggle">
          <input
            type="checkbox"
            checked={settings.current.appearance.showAge}
            onchange={(e) => patch({ appearance: { showAge: e.currentTarget.checked } })}
          />
          <span>{t('settings.showAge')}</span>
        </label>

        <!-- Only while the badge is on: a format for something the cards are
             not showing is a control with nothing to change. -->
        {#if settings.current.appearance.showAge}
          <div class="field">
            <label class="field-label">
              {t('settings.timeFormat')}
              <select
                value={settings.current.appearance.timeFormat}
                onchange={(e) =>
                  patch({ appearance: { timeFormat: e.currentTarget.value as TimeFormat } })}
              >
                <option value="relative">{t('settings.timeFormatRelative')}</option>
                <option value="clock24">{t('settings.timeFormatClock24')}</option>
                <option value="clock12">{t('settings.timeFormatClock12')}</option>
              </select>
            </label>
            <p class="hint">{t('settings.timeFormatHint')}</p>
          </div>
        {/if}

        <div class="field">
          <label class="field-label">
            {t('settings.formatLabel')}
            <select
              value={settings.current.appearance.formatLabelSize}
              onchange={(e) =>
                patch({
                  appearance: {
                    formatLabelSize: e.currentTarget.value as 'off' | 'small' | 'medium' | 'large',
                  },
                })}
            >
              <option value="off">{t('settings.sizeOff')}</option>
              <option value="small">{t('settings.sizeSmall')}</option>
              <option value="medium">{t('settings.sizeMedium')}</option>
              <option value="large">{t('settings.sizeLarge')}</option>
            </select>
          </label>
        </div>

        <label class="toggle">
          <input
            type="checkbox"
            checked={settings.current.appearance.animateGifs}
            onchange={(e) => patch({ appearance: { animateGifs: e.currentTarget.checked } })}
          />
          <span>{t('settings.animateGifs')}</span>
        </label>

        <label class="toggle">
          <input
            type="checkbox"
            checked={settings.current.appearance.reduceMotion}
            onchange={(e) => patch({ appearance: { reduceMotion: e.currentTarget.checked } })}
          />
          <span>{t('settings.reduceMotion')}</span>
        </label>

        <div class="field">
          <span class="field-label">{t('settings.accent')}</span>
          <div class="row">
            <input
              type="color"
              value={effectiveAccent}
              aria-label={t('settings.accent')}
              onchange={(e) => patch({ appearance: { accent: e.currentTarget.value } })}
            />
            <code>{settings.current.appearance.accent || effectiveAccent}</code>
            {#if settings.current.appearance.accent}
              <button
                type="button"
                class="linkish"
                onclick={() => patch({ appearance: { accent: '' } })}
              >
                {t('settings.followTheme')}
              </button>
            {/if}
          </div>
          <p class="hint">
            {settings.current.appearance.accent
              ? 'Overriding the theme’s accent.'
              : 'Following the theme’s accent. Pick a colour to override it.'}
          </p>
        </div>
      </section>

    {:else if section === 'privacy'}
      <section>
        <h2>{t('settings.tab.privacy')}</h2>

        <label class="toggle">
          <input
            type="checkbox"
            checked={settings.current.privacy.respectClipboardFlags}
            onchange={(e) => patch({ privacy: { respectClipboardFlags: e.currentTarget.checked } })}
          />
          <span>{t('settings.respectFlags')}</span>
        </label>
        <p class="hint">
          {t('settings.respectFlagsHint')}
        </p>

        <label class="toggle">
          <input
            type="checkbox"
            checked={settings.current.privacy.linkPreviews}
            onchange={(e) => patch({ privacy: { linkPreviews: e.currentTarget.checked } })}
          />
          <span>{t('settings.linkPreviews')}</span>
        </label>
        <p class="hint">
          {t('settings.linkPreviewsHint')}
        </p>

        <div class="field">
          <span class="field-label">{t('settings.blockedProcesses')}</span>
          <p class="hint">{t('settings.blockedHint')}</p>
          <ul class="blocked-list">
            {#each settings.current.privacy.blockedProcesses as p}
              <li>
                <code>{p}</code>
                <button aria-label={`Remove ${p}`} onclick={() => removeBlocked(p)}>×</button>
              </li>
            {/each}
          </ul>
          <div class="row">
            <input
              type="text"
              placeholder={t('settings.processPlaceholder')}
              bind:value={newBlocked}
              onkeydown={(e) => {
                if (e.key === 'Enter') {
                  e.preventDefault()
                  addBlocked()
                }
              }}
            />
            <button onclick={addBlocked}>{t('settings.add')}</button>
          </div>
        </div>
      </section>

    {:else if section === 'data'}
      <section>
        <h2>{t('settings.tab.data')}</h2>

        <div class="field">
          <span class="field-label">{t('settings.export')}</span>
          <p class="hint">
            {t('settings.exportHint')}
          </p>
          <div class="row">
            <button onclick={() => void doExport()}>{t('settings.exportButton')}</button>
          </div>
        </div>

        <div class="field">
          <span class="field-label">{t('settings.import')}</span>
          <div class="radios">
            <label>
              <input
                type="radio"
                name="importMode"
                checked={importMode === 'merge'}
                onchange={() => { importMode = 'merge' }}
              />
              {t('settings.importMerge')}
            </label>
            <label>
              <input
                type="radio"
                name="importMode"
                checked={importMode === 'replace'}
                onchange={() => { importMode = 'replace' }}
              />
              {t('settings.importReplace')}
            </label>
          </div>
          <div class="row">
            <button onclick={() => void doImport()}>{t('settings.importButton')}</button>
          </div>
        </div>

        <div class="field">
          <span class="field-label">{t('settings.clearHistory')}</span>
          <p class="hint">
            {t('settings.clearHistoryHint')}
          </p>
          <div class="row">
            <button class="danger" onclick={clearHistoryNow}>
              {confirmClear ? 'Click again to confirm' : 'Clear history'}
            </button>
          </div>
        </div>

        <div class="field">
          <span class="field-label">{t('settings.resetAll')}</span>
          <p class="hint">
            {t('settings.resetAllHint')}
          </p>
          <div class="row">
            <button class="danger" onclick={resetEverything}>
              {confirmReset ? 'Click again to confirm' : 'Reset everything'}
            </button>
          </div>
        </div>
      </section>

    {:else}
      <section>
        <h2>{t('settings.tab.about')}</h2>
        <p class="about-name">Rebuffer</p>
        <p class="hint">{t('settings.version', { version: APP_VERSION })}</p>
        <p class="hint">
          {t('settings.aboutBody')}
        </p>
      </section>
    {/if}
  </main>
</div>

<style>
  .settings {
    height: 100vh;
    display: flex;
    background: var(--bg);
    color: var(--text-1, #e8eaf0);
    font-family: var(--font-ui, 'Segoe UI', system-ui, sans-serif);
    font-size: 13px;
  }

  nav {
    flex: none;
    width: 180px;
    padding: 16px 8px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    border-inline-end: 1px solid var(--border-1, rgba(255, 255, 255, 0.08));
  }

  nav button {
    text-align: start;
    padding: 8px 12px;
    border: none;
    border-radius: 8px;
    background: transparent;
    color: var(--text-2, #9aa3b2);
    font: inherit;
    cursor: pointer;
  }

  nav button:hover {
    color: var(--text-1, #e8eaf0);
    background: var(--hover);
  }

  nav button.active {
    color: var(--text-1, #e8eaf0);
    background: var(--hover);
  }

  .content {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    padding: 20px 24px 40px;
  }

  .content section {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .content h2 {
    margin: 0 0 4px;
    font-size: 15px;
    font-weight: 600;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .field-label {
    display: flex;
    flex-direction: column;
    gap: 6px;
    color: var(--text-2, #9aa3b2);
    font-size: 12px;
  }

  .field-label .row {
    margin-top: 2px;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .path {
    font-family: var(--font-mono, 'Cascadia Mono', Consolas, monospace);
    font-size: 12px;
    color: var(--text-2, #9aa3b2);
    word-break: break-all;
  }

  input[type='text'],
  select {
    background: var(--input-bg);
    border: 1px solid var(--border-1, rgba(255, 255, 255, 0.12));
    border-radius: 8px;
    color: var(--text-1, #e8eaf0);
    padding: 7px 10px;
    font: inherit;
    outline: none;
    max-width: 220px;
  }

  input[type='text']:focus,
  select:focus {
    border-color: var(--accent);
  }

  button {
    border: 1px solid var(--border-1, rgba(255, 255, 255, 0.12));
    border-radius: 8px;
    padding: 7px 14px;
    background: transparent;
    color: var(--text-1, #e8eaf0);
    font: inherit;
    cursor: pointer;
  }

  button:hover {
    background: var(--hover);
  }

  button.danger {
    color: #f7768e;
    border-color: rgba(247, 118, 142, 0.4);
  }

  .hotkey-box {
    min-width: 140px;
    font-family: var(--font-mono, 'Cascadia Mono', Consolas, monospace);
    font-size: 13px;
    color: var(--text-1, #e8eaf0);
  }

  .toggle {
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
  }

  .toggle input {
    accent-color: var(--accent);
    margin: 0;
  }

  .radios {
    display: flex;
    gap: 18px;
    flex-wrap: wrap;
  }

  .radios label {
    display: flex;
    align-items: center;
    gap: 6px;
    cursor: pointer;
  }

  .radios input {
    accent-color: var(--accent);
    margin: 0;
  }

  .hint,
  .error,
  .ok {
    margin: 0;
    font-size: 12px;
  }

  .hint {
    color: var(--text-2, #9aa3b2);
    max-width: 560px;
    line-height: 1.45;
  }

  .error {
    color: #f7768e;
    max-width: 560px;
    line-height: 1.45;
  }

  .ok {
    color: #9ece6a;
  }

  .usage {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
    border: 1px solid var(--border-1, rgba(255, 255, 255, 0.08));
    border-radius: 12px;
    background: var(--panel);
    max-width: 640px;
  }

  .usage-head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    color: var(--text-2, #9aa3b2);
    font-size: 12px;
  }

  .meter {
    display: flex;
    height: 14px;
    border-radius: 7px;
    overflow: hidden;
    background: var(--input-bg);
  }

  .meter-seg {
    height: 100%;
  }

  .legend {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .legend li {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-2, #9aa3b2);
  }

  .dot {
    flex: none;
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }

  .blocked-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-width: 480px;
  }

  .blocked-list li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 6px 10px;
    border-radius: 8px;
    background: var(--input-bg);
  }

  .blocked-list code {
    font-family: var(--font-mono, 'Cascadia Mono', Consolas, monospace);
    font-size: 12px;
  }

  .blocked-list button {
    padding: 2px 10px;
    line-height: 1.2;
  }

  .about-name {
    margin: 0;
    font-size: 18px;
    font-weight: 600;
  }

  .progress {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    z-index: 40;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 16px;
    background: var(--panel);
    border-bottom: 1px solid var(--border-1, rgba(255, 255, 255, 0.1));
    font-size: 12px;
    color: var(--text-2, #9aa3b2);
  }

  .progress-track {
    height: 6px;
    border-radius: 3px;
    overflow: hidden;
    background: var(--input-bg);
  }

  .progress-fill {
    height: 100%;
    background: var(--accent);
    transition: width 0.2s;
  }

  .error-banner {
    position: fixed;
    top: 10px;
    right: 10px;
    z-index: 50;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 14px;
    border-radius: 10px;
    background: #f7768e;
    color: #0b0d10;
    font-weight: 600;
    box-shadow: 0 6px 24px rgba(0, 0, 0, 0.35);
  }

  .error-banner button {
    border: none;
    border-radius: 6px;
    padding: 3px 10px;
    background: rgba(0, 0, 0, 0.18);
    color: inherit;
    font-weight: 600;
  }

  .linkish {
    border: none;
    background: none;
    padding: 0;
    color: var(--accent);
    font-size: var(--fs-sm);
    cursor: pointer;
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  .linkish:hover {
    color: var(--accent-strong);
  }

  /* One scrollable row rather than a wrapping grid: the themes are a short
     list to skim, and a row keeps the section from dominating the page.

     Deliberately without `scrollbar-width`. Setting it opts the element out of
     the `::-webkit-scrollbar` rules in global.css and back into the operating
     system's own scrollbar, arrow buttons and all — which is why this one strip
     had a grey Windows scrollbar under it while every other scrollable area in
     the app followed the theme. */
  .theme-strip {
    display: flex;
    gap: 10px;
    overflow-x: auto;
    padding: 4px 2px 10px;
  }
</style>