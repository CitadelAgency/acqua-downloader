import { DEFAULTS, shouldCapture, cookieHeader, baseName } from './rules.js'

const HOST = 'com.dlm.host'

async function config() {
  const { cfg } = await chrome.storage.local.get('cfg')
  return { ...DEFAULTS, ...(cfg || {}) }
}

function send(msg) {
  return new Promise((resolve) => {
    chrome.runtime.sendNativeMessage(HOST, msg, (resp) => {
      if (chrome.runtime.lastError) resolve({ ok: false, error: chrome.runtime.lastError.message })
      else resolve(resp || { ok: false, error: 'sem resposta' })
    })
  })
}

// Capture settings (minimum size, types) come from the app: one source of truth.
async function refreshFromApp() {
  const r = await send({ type: 'ping' })
  if (r.ok && r.result?.capture) {
    const cfg = await config()
    cfg.minSize = r.result.capture.minSize
    cfg.types = r.result.capture.types
    await chrome.storage.local.set({ cfg, hostOk: true })
  } else {
    await chrome.storage.local.set({ hostOk: false, hostError: r.error || '' })
  }
  return r
}

chrome.runtime.onStartup.addListener(refreshFromApp)
chrome.runtime.onInstalled.addListener(() => {
  refreshFromApp()
  chrome.contextMenus.create({ id: 'dlm-link', title: 'Download with Acqua Downloader', contexts: ['link'] })
})

// Hand a download to dlm. Only after the app accepted it is the browser's
// copy cancelled; on any failure the browser simply continues.
async function capture(item) {
  const url = item.finalUrl || item.url
  const cookies = cookieHeader(await chrome.cookies.getAll({ url }))
  const r = await send({
    type: 'add',
    url,
    referrer: item.referrer || '',
    cookies,
    userAgent: navigator.userAgent,
    filename: baseName(item.filename),
    totalBytes: item.totalBytes || 0,
    mime: item.mime || '',
  })
  if (r.ok) {
    if (item.id !== undefined) {
      await chrome.downloads.cancel(item.id).catch(() => {})
      await chrome.downloads.erase({ id: item.id }).catch(() => {})
    }
    if (r.result?.relinked) {
      notify('Acqua Downloader recognised the new link and resumed the paused download.')
    }
  } else {
    if (item.id !== undefined) await chrome.downloads.resume(item.id).catch(() => {})
    notify('Could not send to Acqua Downloader: ' + (r.error || ''))
  }
  return r
}

function notify(message, extra = {}) {
  const id = 'dlm-' + Date.now() + Math.random()
  chrome.notifications.create(id, { type: 'basic', iconUrl: 'icons/icon128.png', title: 'Acqua Downloader', message, ...extra })
  return id
}

chrome.downloads.onCreated.addListener(async (item) => {
  const cfg = await config()
  if (!shouldCapture(item, cfg, chrome.runtime.id)) return
  // Hold the browser download while deciding: nothing is lost if dlm says no.
  await chrome.downloads.pause(item.id).catch(() => {})
  if (cfg.mode === 'auto') {
    await capture(item)
    return
  }
  const name = baseName(item.filename) || new URL(item.finalUrl || item.url).pathname.split('/').pop()
  const nid = notify(name, {
    message: `Download "${name}" with Acqua Downloader?`,
    buttons: [{ title: 'Download with Acqua Downloader' }, { title: 'Keep in browser' }],
    requireInteraction: true,
  })
  await chrome.storage.session.set({ [nid]: item })
})

async function decide(nid, useDlm) {
  const got = await chrome.storage.session.get(nid)
  const item = got[nid]
  if (!item) return
  await chrome.storage.session.remove(nid)
  chrome.notifications.clear(nid)
  if (useDlm) await capture(item)
  else await chrome.downloads.resume(item.id).catch(() => {})
}

chrome.notifications.onButtonClicked.addListener((nid, idx) => decide(nid, idx === 0))
chrome.notifications.onClosed.addListener((nid) => decide(nid, false)) // dismissed: keep in browser

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  if (info.menuItemId !== 'dlm-link' || !info.linkUrl) return
  await capture({ url: info.linkUrl, referrer: tab?.url || '' })
})

chrome.runtime.onMessage.addListener((msg, _sender, reply) => {
  if (msg?.type === 'refresh') {
    refreshFromApp().then(reply)
    return true
  }
})
