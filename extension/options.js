import { DEFAULTS } from './rules.js'

const $ = (id) => document.getElementById(id)

$('l-enabled').textContent = 'Send large downloads to Acqua Downloader'
$('l-ask').textContent = 'Ask first (notification)'
$('l-auto').textContent = 'Send automatically'
$('refresh').textContent = 'Check connection to the app'

async function render() {
  const { cfg, hostOk, hostError } = await chrome.storage.local.get(['cfg', 'hostOk', 'hostError'])
  const c = { ...DEFAULTS, ...(cfg || {}) }
  $('enabled').checked = c.enabled
  document.querySelector(`input[name=mode][value=${c.mode}]`).checked = true
  const mb = Math.round(c.minSize / 1048576)
  $('rules').textContent = `Captures files from ${mb} MB of types: ${c.types.join(', ')} (set in the app).`
  const s = $('status')
  if (hostOk) {
    s.textContent = '✓ Connected to Acqua Downloader'
    s.className = 'ok'
  } else {
    s.textContent = 'Acqua Downloader not found. Install the app and open it once (it registers the host). ' + (hostError || '')
    s.className = 'bad'
  }
}

async function save() {
  const { cfg } = await chrome.storage.local.get('cfg')
  const c = { ...DEFAULTS, ...(cfg || {}) }
  c.enabled = $('enabled').checked
  c.mode = document.querySelector('input[name=mode]:checked').value
  await chrome.storage.local.set({ cfg: c })
}

$('enabled').addEventListener('change', save)
for (const r of document.querySelectorAll('input[name=mode]')) r.addEventListener('change', save)
$('refresh').addEventListener('click', async () => {
  await chrome.runtime.sendMessage({ type: 'refresh' })
  render()
})
render()
