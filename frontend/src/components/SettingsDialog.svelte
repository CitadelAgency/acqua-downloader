<script>
  import { t, setLang, languages } from '../lib/i18n.svelte.js'
  import { errText, parseRate, rateText } from '../lib/format.js'
  import { GetSettings, SaveSettings, ChooseDir, GetInfo } from '../lib/backend.js'

  let { onclose, toast } = $props()
  let s = $state(null)
  let info = $state(null)
  let limit = $state('')
  let capture = $state('')
  let types = $state('')

  $effect(() => {
    GetSettings().then((v) => {
      s = v
      limit = rateText(v.globalLimitBps)
      capture = rateText(v.captureMinSize)
      types = (v.captureTypes || []).join(', ')
    })
    GetInfo().then((v) => (info = v))
  })

  async function choose() {
    const d = await ChooseDir()
    if (d) s.dir = d
  }

  async function save() {
    const bps = parseRate(limit)
    const min = parseRate(capture)
    if (bps === null || min === null) {
      toast(t('s_limitHint'), 'err')
      return
    }
    s.globalLimitBps = bps
    s.captureMinSize = min
    s.captureTypes = types.split(/[\s,;]+/).map((x) => x.replace(/^\./, '').toLowerCase()).filter(Boolean)
    s.maxActive = Number(s.maxActive)
    s.maxConns = Number(s.maxConns)
    try {
      await SaveSettings(s)
      setLang(s.lang)
      onclose()
    } catch (e) {
      toast(errText(e), 'err')
    }
  }
</script>

<div class="dialog">
  <h2>{t('settings')}</h2>
  {#if s}
    <div class="grid">
      <label for="dir">{t('s_dir')}</label>
      <div class="inline"><input id="dir" bind:value={s.dir} /><button onclick={choose}>{t('s_choose')}</button></div>

      <label for="ma">{t('s_maxActive')}</label>
      <input id="ma" type="number" min="1" max="20" bind:value={s.maxActive} />

      <label for="mc">{t('s_maxConns')}</label>
      <input id="mc" type="number" min="1" max="32" bind:value={s.maxConns} />

      <label for="gl">{t('s_limit')}</label>
      <input id="gl" placeholder={t('s_limitHint')} bind:value={limit} />

      <label for="sch"><input id="sch" type="checkbox" bind:checked={s.schedule.enabled} /> {t('s_schedule')}</label>
      <div class="inline">
        <input type="time" bind:value={s.schedule.start} disabled={!s.schedule.enabled} />
        <span>{t('s_and')}</span>
        <input type="time" bind:value={s.schedule.end} disabled={!s.schedule.enabled} />
      </div>

      <label for="lang">{t('s_lang')}</label>
      <select id="lang" bind:value={s.lang}>
        {#each languages as l}<option value={l.id}>{l.name}</option>{/each}
      </select>
    </div>

    <label class="check"><input type="checkbox" bind:checked={s.watchClipboard} /> {t('s_clipboard')}</label>
    <label class="check"><input type="checkbox" bind:checked={s.autostart} /> {t('s_autostart')}</label>
    <label class="check"><input type="checkbox" bind:checked={s.notify} /> {t('s_notify')}</label>

    <h3>{t('s_extension')}</h3>
    <div class="grid">
      <label for="cap">{t('s_capture')}</label>
      <input id="cap" bind:value={capture} />
      <label for="ty">{t('s_types')}</label>
      <input id="ty" bind:value={types} />
      {#if info}
        <span>{t('s_extId')}</span>
        <code>{info.extensionId}</code>
        <span></span>
        <span class:ok={info.hostOk} class:bad={!info.hostOk}>{info.hostOk ? '✓ ' + t('s_hostOk') : t('s_hostBad')}</span>
      {/if}
    </div>
  {/if}
  <div class="buttons">
    <button onclick={onclose}>{t('cancel')}</button>
    <button class="primary" onclick={save} disabled={!s}>{t('save')}</button>
  </div>
</div>

<style>
  .dialog { display: flex; flex-direction: column; gap: 14px; }
  h2 { margin: 0; font-size: 22px; }
  h3 { margin: 10px 0 0; font-size: 17px; color: var(--teal); padding-top: 12px; border-top: 1px solid var(--glass-line); }
  .grid { display: grid; grid-template-columns: 230px 1fr; gap: 10px 14px; align-items: center; }
  .grid > label, .grid > span { color: var(--muted); }
  .inline { display: flex; gap: 8px; align-items: center; }
  .inline input:not([type='time']) { flex: 1; min-width: 0; }
  .check { display: flex; gap: 10px; align-items: center; }
  code { font-family: ui-monospace, Consolas, monospace; user-select: text; }
  .ok { color: var(--done); }
  .bad { color: var(--warn); }
  .buttons { display: flex; justify-content: flex-end; gap: 8px; margin-top: 4px; }
  @media (max-width: 640px) { .grid { grid-template-columns: 1fr; } }
</style>
