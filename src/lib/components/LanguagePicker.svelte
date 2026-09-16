<script lang="ts">
  /* A dropdown of our own rather than a native <select>.
     Windows paints a native select's popup itself: it ignores the theme, opens
     in the system's own grey, and on a dark theme it flashes white. This one is
     ordinary markup, so it takes the same tokens as everything else and follows
     the theme the moment it changes. */
  import { LANGUAGE_NAMES, LOCALE_ORDER, t, type LanguageSetting } from '../i18n/index.svelte'

  /* `value` is typed as the plain string the settings carry rather than the
     narrow union: settings.json is a file a person can edit, so the value
     reaching this component is only ever *claimed* to be a language. It is
     narrowed below, and anything unrecognised shows as "same as Windows",
     which is what the backend will have fallen back to anyway. */
  let { value, onselect }: { value: string; onselect: (v: LanguageSetting) => void } = $props()

  const selected = $derived<LanguageSetting>(
    value === 'system' || (LOCALE_ORDER as string[]).includes(value)
      ? (value as LanguageSetting)
      : 'system',
  )

  let open = $state(false)
  let button = $state<HTMLButtonElement | null>(null)
  let list = $state<HTMLDivElement | null>(null)

  const OPTIONS = $derived<LanguageSetting[]>(['system', ...LOCALE_ORDER])

  function label(v: LanguageSetting): string {
    return v === 'system' ? t('settings.languageSystem') : LANGUAGE_NAMES[v]
  }

  function choose(v: LanguageSetting): void {
    onselect(v)
    open = false
    button?.focus()
  }

  /* Escape closes, the arrows walk the list, Enter picks. A menu that can only
     be used with a mouse is a menu half the settings window cannot reach. */
  function onKey(e: KeyboardEvent): void {
    if (!open) {
      if (e.key === 'ArrowDown' || e.key === 'Enter' || e.key === ' ') {
        e.preventDefault()
        open = true
      }
      return
    }
    if (e.key === 'Escape') {
      e.preventDefault()
      open = false
      button?.focus()
      return
    }
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault()
      const i = OPTIONS.indexOf(selected)
      const next = e.key === 'ArrowDown' ? i + 1 : i - 1
      const clamped = Math.max(0, Math.min(OPTIONS.length - 1, next))
      const option = OPTIONS[clamped]
      if (option !== undefined) onselect(option)
    }
  }

  /* Clicking anywhere else closes it. Bound while open only, so a settings
     window that never opens this menu never pays for the listener. */
  $effect(() => {
    if (!open) return
    function away(e: MouseEvent): void {
      const target = e.target as Node | null
      if (target && !button?.contains(target) && !list?.contains(target)) open = false
    }
    document.addEventListener('mousedown', away, true)
    return () => document.removeEventListener('mousedown', away, true)
  })
</script>

<div class="picker">
  <button
    bind:this={button}
    type="button"
    class="trigger"
    class:open
    aria-haspopup="listbox"
    aria-expanded={open}
    onclick={() => (open = !open)}
    onkeydown={onKey}
  >
    <span class="current">{label(selected)}</span>
    <svg class="chev" viewBox="0 0 24 24" width="14" height="14" aria-hidden="true">
      <path
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
        d="M6 9.5 12 15.5 18 9.5"
      />
    </svg>
  </button>

  {#if open}
    <div bind:this={list} class="list" role="listbox" tabindex="-1" onkeydown={onKey}>
      {#each OPTIONS as option}
        <button
          type="button"
          class="option"
          class:selected={option === selected}
          role="option"
          aria-selected={option === selected}
          onclick={() => choose(option)}
        >
          <span class="name">{label(option)}</span>
          {#if option === selected}
            <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true">
              <path
                fill="none"
                stroke="currentColor"
                stroke-width="2.2"
                stroke-linecap="round"
                stroke-linejoin="round"
                d="M5 12.5 10 17.5 19 7"
              />
            </svg>
          {/if}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .picker {
    position: relative;
    display: inline-block;
    min-width: 220px;
  }

  .trigger {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    width: 100%;
    padding: 7px 10px;
    border: 1px solid var(--border-2);
    border-radius: var(--r-md);
    background: var(--surface-2);
    color: var(--text-1);
    font: inherit;
    font-size: var(--fs-sm);
    cursor: pointer;
    transition: background var(--dur-fast) var(--ease), border-color var(--dur-fast) var(--ease);
  }

  .trigger:hover {
    background: var(--surface-4);
  }

  .trigger.open {
    border-color: var(--accent);
  }

  .chev {
    flex: none;
    color: var(--text-3);
    transition: transform var(--dur-fast) var(--ease);
  }

  .trigger.open .chev {
    transform: rotate(180deg);
  }

  .list {
    position: absolute;
    z-index: 40;
    top: calc(100% + 6px);
    left: 0;
    right: 0;
    max-height: 280px;
    overflow-y: auto;
    padding: 4px;
    border: 1px solid var(--border-2);
    border-radius: var(--r-md);
    /* Opaque, not translucent: the list overlaps the theme strip above it, and
       a see-through menu over a row of coloured cards is unreadable. Every
       theme's --surface-3 carries alpha, so it is painted over --bg-0, the one
       fully opaque colour a theme is guaranteed to define. */
    background-color: var(--bg-0);
    background-image: linear-gradient(var(--surface-3), var(--surface-3));
    box-shadow: var(--shadow-2, 0 12px 32px rgba(0, 0, 0, 0.32));
  }

  .option {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    width: 100%;
    padding: 7px 9px;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--text-1);
    font: inherit;
    font-size: var(--fs-sm);
    text-align: start;
    cursor: pointer;
  }

  .option:hover {
    background: var(--overlay-1);
  }

  .option.selected {
    color: var(--accent-strong, var(--accent));
  }

  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
