<script>
  import SegBar from './SegBar.svelte'
  import { t } from '../lib/i18n.svelte.js'
  import { errText, bytes, duration, parseRate, rateText } from '../lib/format.js'
  import { Pause, Resume, Relink, RetryBrowser, ShowInFolder, Restart, Remove, SetPriority, SetLimit, AddMirror, SetHash } from '../lib/backend.js'

  let { task, ask, toast } = $props()

  let menuOpen = $state(false)
  let menuPos = $state({ right: 0, top: 0, bottom: 0, up: false })

  // The row has backdrop-filter, which traps position:fixed descendants, so
  // the menu and its scrim are moved to <body> and placed from the button.
  function portal(node) {
    document.body.appendChild(node)
    return { destroy: () => node.remove() }
  }
  function toggleMenu(e) {
    const r = e.currentTarget.getBoundingClientRect()
    menuPos = { right: window.innerWidth - r.right, top: r.bottom + 4, bottom: window.innerHeight - r.top + 4, up: r.bottom + 340 > window.innerHeight }
    menuOpen = !menuOpen
  }
  let newLink = $state('')
  let relinking = $state(false)
  let relinkErr = $state('')

  let live = $derived(task.live)
  let liveState = $derived(live?.state || '')
  let total = $derived(live ? live.total : task.total)
  let done = $derived(live ? live.done : task.done)
  let running = $derived(task.state === 'downloading')
  let verifying = $derived(running && liveState === 'verifying')
  let barState = $derived(verifying ? 'verifying' : task.state)
  let integrity = $derived(['remote_changed', 'seam_mismatch', 'hash_mismatch', 'bad_content_range', 'not_same_file', 'part_too_large'].includes(task.errCode))
  let canPause = $derived(['downloading', 'queued', 'scheduled', 'waiting_network'].includes(task.state))
  let canResume = $derived(['paused', 'scheduled', 'waiting_network', 'needs_link'].includes(task.state) || (task.state === 'error' && !integrity))
  let name = $derived(live?.name || task.name || task.url)
  // file-type swatch: extension (≤4 chars) tinted by a stable pigment hue
  let ext = $derived((name || '').match(/\.([a-z0-9]{1,4})$/i)?.[1].toUpperCase() || '')
  let hue = $derived.by(() => {
    let h = 0
    for (const c of ext) h = (h * 31 + c.charCodeAt(0)) % 360
    return [228, 184, 40, 340, 262, 160][h % 6]
  })
  let percent = $derived(total > 0 ? Math.floor((done / total) * 100) : 0)

  function statusText() {
    const s = task.state
    if (s === 'downloading') {
      if (verifying) return t('st_verifying') + (total > 0 ? ` · ${Math.floor((live.verified / total) * 100)}%` : '')
      if (liveState === 'starting' || liveState === 'probing') return t('st_probing')
      const parts = [t('st_downloading')]
      if (live) {
        parts.push(live.conns === 1 ? t('conn1') : t('conns', { n: live.conns }))
        if (live.speed > 0) parts.push(bytes(live.speed) + '/s')
        if (live.eta >= 0) parts.push(t('left', { t: duration(live.eta) }))
      }
      return parts.join(' · ')
    }
    if (s === 'error' || s === 'browser_in_use') return task.message || t('st_' + s)
    if (s === 'done') return t('st_done')
    if (s === 'scheduled' && task.startAt) return t('st_scheduled') + ' · ' + new Date(task.startAt).toLocaleString()
    return t('st_' + s)
  }

  function sizeText() {
    if (task.state === 'done') return bytes(total)
    if (total > 0) return `${bytes(done)} ${t('of')} ${bytes(total)}`
    return done > 0 ? bytes(done) : ''
  }

  async function relink() {
    if (!newLink.trim()) return
    relinking = true
    relinkErr = ''
    try {
      await Relink(task.id, newLink.trim())
      newLink = ''
    } catch (e) {
      relinkErr = errText(e)
    }
    relinking = false
  }

  async function act(fn) {
    menuOpen = false
    try {
      await fn()
    } catch (e) {
      toast(errText(e), 'err')
    }
  }

  const menu = () => [
    { label: t('prioUp'), run: () => SetPriority(task.id, (task.priority || 0) + 1) },
    { label: t('prioDown'), run: () => SetPriority(task.id, (task.priority || 0) - 1) },
    { label: t('limit'), run: async () => {
        const v = await ask(t('limitPrompt'), rateText(task.limitBps))
        if (v === null) return
        const bps = parseRate(v)
        if (bps === null) throw new Error(v)
        await SetLimit(task.id, bps)
      } },
    { label: t('mirror'), hide: task.state === 'done', run: async () => {
        const v = await ask(t('mirrorPrompt'), '')
        if (v) await AddMirror(task.id, v.trim())
      } },
    { label: t('hash'), hide: task.state === 'done', run: async () => {
        const v = await ask(t('hashPrompt'), task.expectedHash || '')
        if (v !== null) await SetHash(task.id, v.trim())
      } },
    { sep: true },
    { label: t('restart'), hide: task.state === 'done', run: async () => {
        if ((await ask(t('restartConfirm'))) !== null) await Restart(task.id)
      } },
    { label: t('remove'), run: () => Remove(task.id, false) },
    { label: t('removePartial'), hide: task.state === 'done', danger: true, run: async () => {
        if ((await ask(t('removePartialConfirm'))) !== null) await Remove(task.id, true)
      } },
  ]
</script>

<article class="row glass" class:err={task.state === 'error'} class:warn={task.state === 'needs_link' || task.state === 'browser_in_use'} class:running class:finished={task.state === 'done'} style="--h:{hue}">
  <div class="swatch" aria-hidden="true">
    {#if task.state === 'done'}
      <svg viewBox="0 0 24 24"><path d="M5 12.5l4.5 4.5L19 7.5" /></svg>
    {:else if ext}
      <span>{ext}</span>
    {:else}
      <svg viewBox="0 0 24 24"><path d="M12 4v12M7 11l5 5 5-5" /></svg>
    {/if}
  </div>
  <div class="body">
    <div class="top">
      <div class="name" title={task.finalPath || task.url}>{name}</div>
      {#if running && total > 0}<div class="percent">{percent}%</div>{/if}
      <div class="size">{sizeText()}</div>
    </div>
    <SegBar {total} {done} doneRanges={live?.doneRanges ?? (task.state === 'done' ? [{ Start: 0, End: total }] : task.ranges || [])} segments={live?.segments || []} state={barState} verified={live?.verified || 0} />
    <div class="bottom">
      <div class="status" class:ok={task.state === 'done'}>
        <span>{statusText()}</span>
        {#if task.state === 'done' && task.check?.checked && task.check?.ok}
          <span class="badge ok">✓ {t('verifiedOk', { algo: task.check.algo === 'md5' ? 'MD5' : 'SHA-256', src: task.check.source })}</span>
        {:else if task.state === 'done' && task.sha256}
          <span class="hash" title={task.sha256}>{t('noHash', { sha: task.sha256.slice(0, 16) + '…' })}</span>
        {/if}
        {#if task.noResume && task.state !== 'done'}<span class="badge warn">{t('noResume')}</span>{/if}
        {#if task.priority}<span class="meta">{t('prio', { p: task.priority })}</span>{/if}
        {#if task.limitBps > 0}<span class="meta">{t('limited', { r: rateText(task.limitBps) })}</span>{/if}
        {#if task.mirrors?.length}<span class="meta">{t('mirrors', { n: task.mirrors.length })}</span>{/if}
      </div>
      <div class="actions">
        {#if task.state === 'browser_in_use'}
          <button onclick={() => act(() => RetryBrowser(task.id))}>{t('retry')}</button>
        {/if}
        {#if integrity && task.state === 'error'}
          <button onclick={() => act(async () => { if ((await ask(t('restartConfirm'))) !== null) await Restart(task.id) })}>{t('restart')}</button>
        {/if}
        {#if canPause}
          <button class="icon ghost" title={t('pause')} aria-label={t('pause')} onclick={() => act(() => Pause(task.id))}>
            <svg viewBox="0 0 24 24"><path d="M8 5v14M16 5v14" /></svg>
          </button>
        {:else if canResume}
          <button class="icon ghost" title={t('resume')} aria-label={t('resume')} onclick={() => act(() => Resume(task.id))}>
            <svg viewBox="0 0 24 24"><path d="M7 5l12 7-12 7z" /></svg>
          </button>
        {/if}
        <button class="icon ghost" title={t('showFolder')} aria-label={t('showFolder')} onclick={() => act(() => ShowInFolder(task.id))}>
          <svg viewBox="0 0 24 24"><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" /></svg>
        </button>
        <div class="menuwrap">
          <button class="icon ghost" title={t('more')} aria-label={t('more')} onclick={toggleMenu}>
            <svg viewBox="0 0 24 24"><circle cx="5" cy="12" r="1" /><circle cx="12" cy="12" r="1" /><circle cx="19" cy="12" r="1" /></svg>
          </button>
          {#if menuOpen}
            <!-- svelte-ignore a11y_no_static_element_interactions a11y_click_events_have_key_events -->
            <div class="scrim" use:portal onclick={() => (menuOpen = false)}></div>
            <div class="menu glass strong" role="menu" use:portal style="right:{menuPos.right}px;{menuPos.up ? `bottom:${menuPos.bottom}px` : `top:${menuPos.top}px`}">
              {#each menu().filter((m) => !m.hide) as m}
                {#if m.sep}<hr />{:else}
                  <button role="menuitem" class="ghost" class:danger={m.danger} onclick={() => act(m.run)}>{m.label}</button>
                {/if}
              {/each}
            </div>
          {/if}
        </div>
      </div>
    </div>
    {#if task.state === 'needs_link'}
      <form class="relink" onsubmit={(e) => { e.preventDefault(); relink() }}>
        <input placeholder={t('newLinkPlaceholder')} bind:value={newLink} disabled={relinking} />
        <button class="primary" disabled={relinking || !newLink.trim()}>{relinking ? t('checking') : t('useLink')}</button>
      </form>
      {#if relinkErr}<div class="relinkerr">{relinkErr}</div>{/if}
    {/if}
  </div>
</article>

<style>
  .row { display: flex; gap: 14px; padding: 13px 14px 11px 13px; transition: transform 160ms ease, box-shadow 160ms ease; }
  .row:hover { transform: translateY(-1px); box-shadow: var(--glass-shadow-float); }
  .row.err { background: color-mix(in oklab, var(--err) 9%, var(--glass)); }
  .row.warn { background: color-mix(in oklab, var(--warn) 10%, var(--glass)); }

  /* pigment swatch: a soft wet blot with the extension on top */
  .swatch { flex: none; width: 42px; height: 42px; margin-top: 1px; border-radius: 14px 12px 15px 11px / 12px 15px 11px 14px; display: grid; place-items: center; font-family: var(--font-display); font-size: 12px; letter-spacing: 0.02em; color: hsl(var(--h) 55% 32%); background: radial-gradient(circle at 35% 30%, hsl(var(--h) 70% 82% / 0.95), hsl(var(--h) 60% 70% / 0.75) 60%, hsl(var(--h) 60% 66% / 0.55)); box-shadow: 0 1px 0 rgb(255 255 255 / 60%) inset, 0 4px 10px -6px hsl(var(--h) 60% 40% / 0.6); }
  .swatch svg { width: 18px; height: 18px; stroke-width: 2.2; }
  .finished .swatch { color: #fff; background: radial-gradient(circle at 35% 30%, color-mix(in oklab, var(--teal) 70%, white), var(--teal) 70%); }
  .err .swatch { color: #fff; background: radial-gradient(circle at 35% 30%, color-mix(in oklab, var(--err) 70%, white), var(--err) 70%); }
  @media (prefers-color-scheme: dark) {
    .swatch { color: hsl(var(--h) 80% 85%); background: radial-gradient(circle at 35% 30%, hsl(var(--h) 55% 45% / 0.9), hsl(var(--h) 50% 32% / 0.8) 60%, hsl(var(--h) 50% 28% / 0.7)); box-shadow: 0 1px 0 rgb(255 255 255 / 12%) inset; }
  }

  .body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 8px; }
  .top { display: flex; gap: 12px; align-items: baseline; }
  .name { flex: 1; font-weight: 600; font-size: 15.5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; user-select: text; }
  .percent { font-family: var(--font-display); font-size: 17px; color: var(--accent); font-variant-numeric: tabular-nums; }
  .size { color: var(--muted); font-variant-numeric: tabular-nums; white-space: nowrap; font-size: 14px; }
  .bottom { display: flex; align-items: center; gap: 8px; min-height: 30px; }
  .status { flex: 1; color: var(--muted); display: flex; flex-wrap: wrap; gap: 4px 10px; align-items: center; font-variant-numeric: tabular-nums; font-size: 14px; }
  .err .status { color: var(--err); }
  .warn .status { color: var(--warn); }
  .status.ok { color: var(--done); }
  .badge { font-size: 13px; padding: 2px 10px; border-radius: 99px; }
  .badge.ok { background: color-mix(in oklab, var(--done) 15%, transparent); color: var(--done); }
  .badge.warn { background: color-mix(in oklab, var(--warn) 15%, transparent); color: var(--warn); }
  .hash { font-size: 13px; color: var(--muted); font-family: "Cascadia Mono", ui-monospace, Consolas, monospace; user-select: text; }
  .meta { font-size: 13px; }
  .actions { display: flex; gap: 2px; align-items: center; opacity: 0.8; transition: opacity 120ms ease; }
  .row:hover .actions, .row:focus-within .actions { opacity: 1; }
  .scrim { position: fixed; inset: 0; z-index: 29; }
  .menu { position: fixed; z-index: 30; min-width: 250px; border-radius: 14px; padding: 5px; display: flex; flex-direction: column; animation: rise 160ms ease both; }
  .menu button { text-align: left; padding: 8px 12px; border-radius: 9px; font-weight: 500; }
  .menu .danger { color: var(--err); }
  .menu hr { border: 0; border-top: 1px solid var(--glass-line); margin: 4px 6px; }
  .relink { display: flex; gap: 8px; }
  .relink input { flex: 1; }
  .relinkerr { color: var(--err); font-size: 13px; user-select: text; }
</style>
