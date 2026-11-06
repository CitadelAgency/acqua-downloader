<script>
  import { t } from '../lib/i18n.svelte.js'
  import { errText, bytes } from '../lib/format.js'
  import { ChromeList, ChromeImport } from '../lib/backend.js'

  let { onclose, toast } = $props()
  let items = $state(null)
  let picked = $state({})
  let busy = $state(false)
  let error = $state('')

  $effect(() => {
    ChromeList()
      .then((list) => {
        items = list || []
        for (const [i, it] of items.entries()) if (!it.problem) picked[i] = true
      })
      .catch((e) => (error = errText(e)))
  })

  let count = $derived(Object.values(picked).filter(Boolean).length)

  async function go() {
    busy = true
    const sel = items.filter((_, i) => picked[i])
    let errs = []
    try {
      errs = await ChromeImport(sel)
    } catch (e) {
      errs = [errText(e)]
    }
    busy = false
    for (const e of errs || []) toast(e, 'err')
    if (!errs || errs.length < sel.length) onclose()
  }
</script>

<div class="dialog">
  <h2>{t('importTitle')}</h2>
  <p class="hint">{t('importHint')}</p>
  {#if error}
    <p class="error">{error}</p>
  {:else if items === null}
    <p class="hint">{t('importLoading')}</p>
  {:else if items.length === 0}
    <p class="hint">{t('importNone')}</p>
  {:else}
    <ul>
      {#each items as it, i}
        <li class:disabled={!!it.problem}>
          <label>
            <input type="checkbox" bind:checked={picked[i]} disabled={!!it.problem} />
            <span class="main">
              <span class="file">{it.targetPath.split('\\').pop()}</span>
              <span class="sub">
                {it.browser} · {it.profile} ·
                {#if it.problem}{it.problem}{:else}{bytes(it.resumeAt)} {t('of')} {bytes(it.total)} {t('received')}{#if it.inUse} · {t('importInUse')}{/if}{/if}
              </span>
            </span>
          </label>
        </li>
      {/each}
    </ul>
  {/if}
  <div class="buttons">
    <button onclick={onclose}>{t('close')}</button>
    {#if items?.length}
      <button class="primary" disabled={busy || count === 0} onclick={go}>{t('importBtn')} ({count})</button>
    {/if}
  </div>
</div>

<style>
  .dialog { display: flex; flex-direction: column; gap: 12px; max-height: 80vh; }
  h2 { margin: 0; font-size: 22px; }
  .hint { margin: 0; color: var(--muted); line-height: 1.5; }
  .error { color: var(--err); }
  ul { list-style: none; margin: 0; padding: 4px; overflow: auto; border: 1px solid var(--glass-line); border-radius: 14px; background: rgb(255 255 255 / 30%); }
  @media (prefers-color-scheme: dark) { ul { background: rgb(0 0 0 / 18%); } }
  li + li { border-top: 1px solid var(--glass-line); }
  li.disabled { opacity: 0.6; }
  label { display: flex; gap: 12px; padding: 10px 12px; align-items: flex-start; cursor: pointer; border-radius: 10px; }
  label:hover { background: rgb(255 255 255 / 40%); }
  @media (prefers-color-scheme: dark) { label:hover { background: rgb(255 255 255 / 6%); } }
  label input { margin-top: 3px; }
  .main { display: flex; flex-direction: column; min-width: 0; }
  .file { font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub { color: var(--muted); font-size: 13px; }
  .buttons { display: flex; justify-content: flex-end; gap: 8px; }
</style>
