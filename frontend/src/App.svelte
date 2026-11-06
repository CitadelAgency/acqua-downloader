<script>
  import TaskRow from './components/TaskRow.svelte'
  import ImportDialog from './components/ImportDialog.svelte'
  import SettingsDialog from './components/SettingsDialog.svelte'
  import WindowControls from './components/WindowControls.svelte'
  import { t, setLang } from './lib/i18n.svelte.js'
  import { errText, bytes } from './lib/format.js'
  import { List, AddText, PauseAll, ResumeAll, GetSettings, EventsOn } from './lib/backend.js'
  import mark from './assets/mark.webp'
  import empty from './assets/empty.webp'

  let tasks = $state([])
  let link = $state('')
  let dragging = $state(false)
  let dialog = $state('') // '', 'import', 'settings'
  let toasts = $state([])
  let askState = $state(null)

  let totalSpeed = $derived(tasks.reduce((a, x) => a + (x.live?.speed || 0), 0))
  let active = $derived(tasks.filter((x) => ['downloading', 'queued', 'waiting_network', 'scheduled'].includes(x.state)).length)
  let anyActive = $derived(active > 0)

  $effect(() => {
    GetSettings().then((s) => setLang(s.lang))
    List().then((v) => (tasks = v || []))
    return EventsOn('tasks', (v) => (tasks = v || []))
  })

  function toast(text, kind = 'info') {
    const id = Math.random()
    toasts.push({ id, text, kind })
    setTimeout(() => (toasts = toasts.filter((x) => x.id !== id)), kind === 'err' ? 9000 : 3500)
  }

  // ask(text) -> confirm (resolves '' or null); ask(text, value) -> prompt
  function ask(text, value) {
    return new Promise((resolve) => {
      askState = { text, value, input: value !== undefined, resolve }
    })
  }
  function answer(ok) {
    const a = askState
    askState = null
    a.resolve(ok ? (a.input ? a.value : '') : null)
  }

  async function addText(text) {
    if (!text || !text.trim()) return
    try {
      const res = (await AddText(text)) || []
      for (const r of res) toast(r.relinked ? t('relinked') : r.existing ? t('existing') : t('added'))
      link = ''
    } catch (e) {
      toast(errText(e), 'err')
    }
  }

  function onPaste(e) {
    const el = document.activeElement
    if (el && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA')) return
    const text = e.clipboardData?.getData('text')
    if (text) {
      e.preventDefault()
      addText(text)
    }
  }

  function onDrop(e) {
    e.preventDefault()
    dragging = false
    const dt = e.dataTransfer
    addText(dt.getData('text/uri-list') || dt.getData('text/plain') || dt.getData('URL'))
  }
</script>

<svelte:window
  onpaste={onPaste}
  ondragover={(e) => { e.preventDefault(); dragging = true }}
  ondragleave={(e) => { if (!e.relatedTarget) dragging = false }}
  ondrop={onDrop}
/>

<div class="paper"></div>

<main>
  <header data-tauri-drag-region>
    <div class="brand" data-tauri-drag-region>
      <img src={mark} alt="" width="34" height="34" draggable="false" data-tauri-drag-region />
      <h1 data-tauri-drag-region>Acqua Downloader</h1>
      {#if anyActive}<span class="pill" data-tauri-drag-region>{active}</span>{/if}
    </div>
    <div class="tools">
      <button class="ghost import-browser" title={t('importChrome')} aria-label={t('importChrome')} onclick={() => (dialog = 'import')}>
        <svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="9" /><circle cx="12" cy="12" r="3.5" /><path d="M12 3v5.5M20 17l-4.8-2.7M4 17l4.8-2.7" /></svg>
        <span class="import-label">{t('importChrome')}</span>
      </button>
      {#if anyActive}
        <button class="icon ghost" title={t('pauseAll')} aria-label={t('pauseAll')} onclick={PauseAll}><svg viewBox="0 0 24 24"><path d="M8 5v14M16 5v14" /></svg></button>
      {:else}
        <button class="icon ghost" title={t('resumeAll')} aria-label={t('resumeAll')} onclick={ResumeAll}><svg viewBox="0 0 24 24"><path d="M7 5l12 7-12 7z" /></svg></button>
      {/if}
      <button class="icon ghost" title={t('settings')} aria-label={t('settings')} onclick={() => (dialog = 'settings')}>
        <svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z" /></svg>
      </button>
    </div>
    <WindowControls onerror={(e) => toast(errText(e), 'err')} />
  </header>

  <form class="add glass" onsubmit={(e) => { e.preventDefault(); addText(link) }}>
    <svg class="lead" viewBox="0 0 24 24"><path d="M10 14a3.5 3.5 0 0 0 5 0l3-3a3.5 3.5 0 0 0-5-5l-1 1M14 10a3.5 3.5 0 0 0-5 0l-3 3a3.5 3.5 0 0 0 5 5l1-1" /></svg>
    <input class="link" placeholder={t('pastePlaceholder')} bind:value={link} spellcheck="false" />
    <button class="primary" disabled={!link.trim()}>{t('download')}</button>
  </form>

  <section class="list">
    {#if tasks.length === 0}
      <div class="empty">
        <img src={empty} alt="" width="200" height="200" draggable="false" />
        <h2>{t('emptyTitle')}</h2>
        <p>{t('emptyHint')}</p>
      </div>
    {:else}
      {#each tasks as task, i (task.id)}
        <div class="rise" style="animation-delay:{Math.min(i, 12) * 35}ms">
          <TaskRow {task} {ask} {toast} />
        </div>
      {/each}
    {/if}
  </section>

  {#if totalSpeed > 0}
    <footer class="glass">
      <svg viewBox="0 0 24 24"><path d="M12 4v12M7 11l5 5 5-5" /></svg>
      <span>{bytes(totalSpeed)}/s</span>
    </footer>
  {/if}
</main>

{#if dragging}
  <div class="drop"><div class="glass strong">{t('dropHere')}</div></div>
{/if}

{#if dialog}
  <div class="modal" role="dialog" aria-modal="true">
    <div class="card glass strong">
      {#if dialog === 'import'}
        <ImportDialog onclose={() => (dialog = '')} {toast} />
      {:else}
        <SettingsDialog onclose={() => (dialog = '')} {toast} />
      {/if}
    </div>
  </div>
{/if}

{#if askState}
  <div class="modal" role="dialog" aria-modal="true">
    <form class="card ask glass strong" onsubmit={(e) => { e.preventDefault(); answer(true) }}>
      <p>{askState.text}</p>
      {#if askState.input}
        <!-- svelte-ignore a11y_autofocus -->
        <input bind:value={askState.value} autofocus spellcheck="false" />
      {/if}
      <div class="buttons">
        <button type="button" onclick={() => answer(false)}>{t('cancel')}</button>
        <button class="primary">{t('ok')}</button>
      </div>
    </form>
  </div>
{/if}

<div class="toasts" aria-live="polite">
  {#each toasts as x (x.id)}
    <div class="toast" class:err={x.kind === 'err'}>{x.text}</div>
  {/each}
</div>

<style>
  main { height: 100%; display: flex; flex-direction: column; padding: 0 clamp(14px, 4vw, 40px); }

  /* Right padding keeps the tools clear of the window controls in the corner. */
  header { display: flex; align-items: center; gap: 12px; padding: 18px calc(150px - clamp(14px, 4vw, 40px)) 12px 2px; }
  .brand { display: flex; align-items: center; gap: 10px; flex: 1; min-width: 0; }
  .brand img { filter: drop-shadow(0 2px 4px rgb(28 34 51 / 15%)); }
  .brand h1 { margin: 0; font-size: 26px; line-height: 1; letter-spacing: -0.02em; }
  .pill { font-size: 12px; font-weight: 600; font-variant-numeric: tabular-nums; color: var(--accent); background: color-mix(in oklab, var(--accent) 14%, transparent); border-radius: 99px; padding: 2px 9px; }
  .tools { display: flex; gap: 2px; align-items: center; }
  .tools button:not(.icon) { display: inline-flex; align-items: center; gap: 7px; }
  @media (max-width: 680px) {
    .tools .import-label { display: none; }
    .tools .import-browser { width: 34px; height: 34px; padding: 6px; justify-content: center; }
  }

  .add { display: flex; align-items: center; gap: 8px; padding: 6px 6px 6px 14px; border-radius: 999px; margin-bottom: 14px; animation: rise 320ms ease both; }
  .add .lead { color: var(--faint); flex: none; }
  .add .link { flex: 1; min-width: 0; background: transparent; border: 0; box-shadow: none; padding: 7px 4px; font-size: 16px; }
  .add .link:focus { outline: none; }
  .add .primary { border-radius: 999px; padding: 8px 18px; }

  .list { flex: 1; overflow: auto; padding: 2px 2px 20px; display: flex; flex-direction: column; gap: 10px; margin: 0 -2px; }
  .rise { animation: rise 360ms cubic-bezier(.2,.7,.2,1) both; }

  .empty { margin: auto; text-align: center; color: var(--muted); max-width: 400px; padding-bottom: 6vh; animation: rise 500ms ease both; }
  .empty img { opacity: 0.95; }
  .empty h2 { font-size: 24px; color: var(--ink); margin: 2px 0 6px; }
  .empty p { margin: 0; line-height: 1.5; font-size: 15px; }

  footer { position: absolute; right: clamp(14px, 4vw, 40px); bottom: 14px; display: flex; align-items: center; gap: 8px; padding: 7px 14px 7px 10px; border-radius: 999px; color: var(--ink); font-variant-numeric: tabular-nums; font-weight: 600; animation: rise 240ms ease both; }
  footer svg { color: var(--teal); }

  .drop { position: fixed; inset: 0; background: color-mix(in oklab, var(--accent) 10%, transparent); display: grid; place-items: center; pointer-events: none; z-index: 50; }
  .drop div { border: 2px dashed color-mix(in oklab, var(--accent) 60%, transparent); padding: 30px 50px; font-size: 22px; font-family: var(--font-display); color: var(--accent); }

  .modal { position: fixed; inset: 0; background: rgb(20 24 40 / 28%); -webkit-backdrop-filter: blur(6px); backdrop-filter: blur(6px); display: grid; place-items: center; z-index: 40; padding: 16px; animation: fade 160ms ease both; }
  @keyframes fade { from { opacity: 0; } }
  .card { padding: 22px; width: min(640px, 100%); max-height: 90vh; overflow: auto; border-radius: 20px; animation: rise 220ms cubic-bezier(.2,.7,.2,1) both; }
  .ask { display: flex; flex-direction: column; gap: 14px; width: min(460px, 100%); }
  .ask p { margin: 0; font-size: 15px; line-height: 1.5; }
  .buttons { display: flex; justify-content: flex-end; gap: 8px; }

  .toasts { position: fixed; bottom: 16px; left: 50%; transform: translateX(-50%); display: flex; flex-direction: column; gap: 6px; z-index: 60; width: min(520px, calc(100% - 32px)); pointer-events: none; }
  .toast { background: var(--ink); color: var(--paper); padding: 10px 16px; border-radius: 12px; box-shadow: 0 12px 30px -12px rgb(0 0 0 / 45%); user-select: text; pointer-events: auto; text-align: center; animation: rise 220ms ease both; }
  .toast.err { background: var(--err); color: #fff; }
</style>
