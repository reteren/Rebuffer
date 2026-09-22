<script lang="ts">
  /* A number input whose steppers are ours. The native
     ::-webkit-inner-spin-button is a grey Windows relic: it ignores every theme
     token, only appears on hover and sits at a size nothing else in the app
     uses. We hide it and draw the two chevrons from the same surfaces, borders
     and accent every other control reads. */
  interface Props {
    value: number | null
    min?: number
    max?: number
    step?: number
    ariaLabel?: string
    /** Empty input commits null instead of clamping up to `min`. */
    allowEmpty?: boolean
    width?: string
    onchange: (n: number | null) => void
  }

  let {
    value,
    min = Number.NEGATIVE_INFINITY,
    max = Number.POSITIVE_INFINITY,
    step = 1,
    ariaLabel,
    allowEmpty = false,
    width = '110px',
    onchange,
  }: Props = $props()

  /* Hold-to-repeat: a pause before the run starts so a single click stays a
     single step, then a steady run that tightens once it is clearly a hold. */
  const HOLD_DELAY = 400
  const REPEAT_MS = 70
  const FAST_AFTER = 8
  const FAST_MS = 35

  // svelte-ignore state_referenced_locally
  let text = $state(value === null ? '' : String(value))
  let editing = $state(false)
  let holdTimer: ReturnType<typeof setTimeout> | null = null
  let repeats = 0

  /* While the field has focus the text belongs to the person typing; outside of
     that it follows the store, so a reset or an external settings change shows
     up here. */
  $effect(() => {
    const incoming = value === null ? '' : String(value)
    if (!editing) text = incoming
  })

  /* -Infinity is not a valid min= / max= attribute, so an unbounded side gets
     no attribute at all rather than a string the browser throws away. */
  const minAttr = $derived(Number.isFinite(min) ? min : undefined)
  const maxAttr = $derived(Number.isFinite(max) ? max : undefined)

  const current = $derived(Number(text))
  const atMin = $derived(Number.isFinite(current) && text !== '' && current <= min)
  const atMax = $derived(Number.isFinite(current) && text !== '' && current >= max)

  function clamp(n: number): number {
    return Math.max(min, Math.min(max, Math.round(n)))
  }

  function commit(raw: string): void {
    if (raw.trim() === '' && allowEmpty) {
      text = ''
      onchange(null)
      return
    }
    const n = Number(raw)
    const next = clamp(Number.isFinite(n) ? n : min)
    text = String(next)
    onchange(next)
  }

  function nudge(dir: 1 | -1): void {
    const base = Number.isFinite(current) && text !== '' ? current : min
    const next = clamp((Number.isFinite(base) ? base : 0) + dir * step)
    if (String(next) === text) return
    text = String(next)
    onchange(next)
  }

  function stopHold(): void {
    if (holdTimer !== null) clearTimeout(holdTimer)
    holdTimer = null
    repeats = 0
  }

  function runHold(dir: 1 | -1): void {
    repeats += 1
    nudge(dir)
    holdTimer = setTimeout(() => runHold(dir), repeats >= FAST_AFTER ? FAST_MS : REPEAT_MS)
  }

  function onStepDown(e: PointerEvent, dir: 1 | -1): void {
    if (e.button !== 0) return
    ;(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId)
    nudge(dir)
    stopHold()
    holdTimer = setTimeout(() => runHold(dir), HOLD_DELAY)
  }

  /* The wheel over a focused number input silently rewrites the setting the
     person was only scrolling past. Dropping focus lets the panel scroll and
     leaves the value alone. */
  function onWheel(e: WheelEvent): void {
    const el = e.currentTarget as HTMLInputElement
    if (document.activeElement === el) el.blur()
  }
</script>

<div class="numfield" style:width>
  <input
    type="number"
    min={minAttr}
    max={maxAttr}
    {step}
    value={text}
    aria-label={ariaLabel}
    oninput={(e) => { text = e.currentTarget.value }}
    onfocus={() => { editing = true }}
    onblur={(e) => { editing = false; commit(e.currentTarget.value) }}
    onchange={(e) => commit(e.currentTarget.value)}
    onwheel={onWheel}
  />
  <div class="steps">
    <button
      type="button"
      class="step up"
      tabindex="-1"
      aria-hidden="true"
      disabled={atMax}
      onpointerdown={(e) => onStepDown(e, 1)}
      onpointerup={stopHold}
      onpointercancel={stopHold}
      onlostpointercapture={stopHold}
    >
      <svg viewBox="0 0 10 6" aria-hidden="true">
        <path d="M1 5l4-4 4 4" fill="none" stroke="currentColor" stroke-width="1.6"
          stroke-linecap="round" stroke-linejoin="round" />
      </svg>
    </button>
    <button
      type="button"
      class="step down"
      tabindex="-1"
      aria-hidden="true"
      disabled={atMin}
      onpointerdown={(e) => onStepDown(e, -1)}
      onpointerup={stopHold}
      onpointercancel={stopHold}
      onlostpointercapture={stopHold}
    >
      <svg viewBox="0 0 10 6" aria-hidden="true">
        <path d="M1 1l4 4 4-4" fill="none" stroke="currentColor" stroke-width="1.6"
          stroke-linecap="round" stroke-linejoin="round" />
      </svg>
    </button>
  </div>
</div>

<style>
  .numfield {
    display: inline-flex;
    align-items: stretch;
    background: var(--input-bg);
    border: 1px solid var(--border-1);
    border-radius: 8px;
    overflow: hidden;
    transition: border-color var(--dur-fast) var(--ease-out);
  }

  .numfield:hover {
    border-color: var(--border-2);
  }

  .numfield:focus-within {
    border-color: var(--accent);
  }

  input {
    flex: 1;
    min-width: 0;
    padding: 7px 6px 7px 10px;
    border: 0;
    background: transparent;
    color: var(--text-1);
    font: inherit;
    outline: none;
    appearance: textfield;
    -moz-appearance: textfield;
  }

  /* The grey Windows spinner, gone. */
  input::-webkit-inner-spin-button,
  input::-webkit-outer-spin-button {
    -webkit-appearance: none;
    appearance: none;
    margin: 0;
  }

  .steps {
    display: flex;
    flex-direction: column;
    width: 22px;
    flex: none;
    border-left: 1px solid var(--border-1);
  }

  .step {
    flex: 1;
    display: grid;
    place-items: center;
    padding: 0;
    border: 0;
    border-radius: 0;
    background: transparent;
    color: var(--text-3);
    cursor: pointer;
    transition:
      background var(--dur-fast) var(--ease-out),
      color var(--dur-fast) var(--ease-out);
  }

  .step.up {
    border-bottom: 1px solid var(--border-1);
  }

  .step:hover:not(:disabled) {
    background: var(--overlay-1);
    color: var(--text-1);
  }

  .step:active:not(:disabled) {
    background: var(--accent-soft);
    color: var(--accent);
  }

  .step:disabled {
    color: var(--text-3);
    opacity: 0.35;
    cursor: default;
  }

  .step svg {
    width: 9px;
    height: 6px;
    display: block;
  }
</style>
