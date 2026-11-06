<script>
  import { onMount } from 'svelte'
  import { t } from '../lib/i18n.svelte.js'

  let { onerror } = $props()
  let appWindow = $state(null)
  let maximized = $state(false)
  let focused = $state(true)

  onMount(() => {
    const current = window.__TAURI__?.window.getCurrentWindow()
    if (!current) return
    appWindow = current
    let disposed = false
    const listeners = []
    const keep = (stop) => disposed ? stop() : listeners.push(stop)
    const syncMaximized = async () => {
      try { const value = await current.isMaximized(); if (!disposed) maximized = value }
      catch (e) { if (!disposed) onerror?.(e) }
    }
    syncMaximized()
    current.isFocused().then((value) => { if (!disposed) focused = value }).catch(() => {})
    current.onResized(syncMaximized).then(keep).catch((e) => onerror?.(e))
    current.onFocusChanged(({ payload }) => { focused = payload }).then(keep).catch((e) => onerror?.(e))
    return () => { disposed = true; for (const stop of listeners) stop() }
  })

  async function act(action) {
    if (!appWindow) return
    try {
      await appWindow[action]()
      if (action === 'toggleMaximize') maximized = await appWindow.isMaximized()
    } catch (e) { onerror?.(e) }
  }
</script>

<div class="window-controls" class:unfocused={!focused} role="group" aria-label={t('windowControls')}>
  <button class="window-button" type="button" title={t('minimize')} aria-label={t('minimize')} onclick={() => act('minimize')}>
    <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4 10.5h8" /></svg>
  </button>
  <button class="window-button" type="button" title={t(maximized ? 'restoreWindow' : 'maximize')} aria-label={t(maximized ? 'restoreWindow' : 'maximize')} onclick={() => act('toggleMaximize')}>
    <svg viewBox="0 0 16 16" aria-hidden="true">
      {#if maximized}
        <path d="M5.5 4V3.5a1 1 0 0 1 1-1H12a1.5 1.5 0 0 1 1.5 1.5v5.5a1 1 0 0 1-1 1H12" />
        <rect x="2.5" y="5.5" width="8" height="8" rx="1.6" />
      {:else}
        <rect x="3.5" y="3.5" width="9" height="9" rx="2" />
      {/if}
    </svg>
  </button>
  <button class="window-button close" type="button" title={t('closeWindow')} aria-label={t('closeWindow')} onclick={() => act('close')}>
    <svg viewBox="0 0 16 16" aria-hidden="true"><path d="m4.5 4.5 7 7m0-7-7 7" /></svg>
  </button>
</div>

<style>
  .window-controls {
    position: relative;
    z-index: 70;
    display: flex;
    flex: none;
    align-items: center;
    gap: 3px;
    padding: 3px;
    margin-left: 9px;
    border: 1px solid var(--glass-line);
    border-radius: 13px;
    background: color-mix(in oklab, var(--paper) 45%, transparent);
    box-shadow: 0 1px 0 var(--glass-edge) inset;
    transition: opacity 150ms ease;
  }
  .window-controls.unfocused { opacity: .6; }
  .window-controls:hover, .window-controls:focus-within { opacity: 1; }
  .window-button {
    display: grid;
    place-items: center;
    width: 32px;
    height: 30px;
    padding: 0;
    border: 0;
    border-radius: 9px;
    background: transparent;
    color: var(--muted);
    box-shadow: none;
    transform: none;
    cursor: default;
    transition: color 130ms ease, background 130ms ease;
  }
  .window-button svg { width: 16px; height: 16px; fill: none; stroke: currentColor; stroke-width: 1.35; stroke-linecap: round; stroke-linejoin: round; pointer-events: none; }
  .window-button:hover { background: color-mix(in oklab, var(--accent) 12%, transparent); color: var(--accent); transform: none; }
  .window-button:active { background: color-mix(in oklab, var(--accent) 20%, transparent); }
  .window-button.close:hover { background: color-mix(in oklab, var(--rose) 23%, transparent); color: var(--err); }
  .window-button.close:active { background: color-mix(in oklab, var(--rose) 35%, transparent); }
  .window-button:focus-visible { outline-offset: -3px; }
  @media (prefers-reduced-motion: reduce) { .window-controls, .window-button { transition: none; } }
</style>
