import { DEFAULTS } from './rules.js'

const pt = (navigator.language || '').toLowerCase().startsWith('pt')
const t = (a, b) => (pt ? a : b)
const $ = (id) => document.getElementById(id)

$('l-enabled').textContent = t('Enviar downloads grandes para o dlm', 'Send large downloads to dlm')
$('l-ask').textContent = t('Perguntar antes (notificação)', 'Ask first (notification)')
$('l-auto').textContent = t('Enviar automaticamente', 'Send automatically')
$('refresh').textContent = t('Verificar conexão com o app', 'Check connection to the app')

async function render() {
  const { cfg, hostOk, hostError } = await chrome.storage.local.get(['cfg', 'hostOk', 'hostError'])
  const c = { ...DEFAULTS, ...(cfg || {}) }
  $('enabled').checked = c.enabled
  document.querySelector(`input[name=mode][value=${c.mode}]`).checked = true
  const mb = Math.round(c.minSize / 1048576)
  $('rules').textContent = t(`Captura arquivos a partir de ${mb} MB dos tipos: ${c.types.join(', ')} (definido no app).`,
    `Captures files from ${mb} MB of types: ${c.types.join(', ')} (set in the app).`)
  const s = $('status')
  if (hostOk) {
    s.textContent = t('✓ Conectado ao dlm', '✓ Connected to dlm')
    s.className = 'ok'
  } else {
    s.textContent = t('dlm não encontrado. Instale o app e abra-o uma vez (ele registra o host). ', 'dlm not found. Install the app and open it once (it registers the host). ') + (hostError || '')
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
