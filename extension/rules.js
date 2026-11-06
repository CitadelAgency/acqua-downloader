// Pure capture rules, shared by the service worker and the tests (node --test).

export const DEFAULTS = {
  enabled: true,
  mode: 'ask', // 'ask' (notification with buttons) or 'auto'
  minSize: 5 * 1024 * 1024,
  types: ['zip', 'rar', '7z', 'iso', 'exe', 'msi', 'mp4', 'mkv', 'pdf', 'gz', 'xz', 'bin', 'img', 'dmg', 'apk'],
}

const MIME_EXT = {
  'application/zip': 'zip',
  'application/x-zip-compressed': 'zip',
  'application/vnd.rar': 'rar',
  'application/x-rar-compressed': 'rar',
  'application/x-7z-compressed': '7z',
  'application/x-iso9660-image': 'iso',
  'application/x-msdownload': 'exe',
  'application/x-msi': 'msi',
  'video/mp4': 'mp4',
  'video/x-matroska': 'mkv',
  'application/pdf': 'pdf',
  'application/gzip': 'gz',
  'application/x-xz': 'xz',
  'application/vnd.android.package-archive': 'apk',
}

export function baseName(p) {
  return (p || '').split(/[\\/]/).pop()
}

function extFromName(n) {
  const m = (n || '').match(/\.([a-z0-9]{1,8})$/i)
  return m ? m[1].toLowerCase() : ''
}

// extOf returns the lower-case extension from the file name, the URL path or
// the MIME type, in that order ('' if unknown).
export function extOf(item) {
  let e = extFromName(baseName(item.filename))
  if (!e) {
    try {
      e = extFromName(decodeURIComponent(new URL(item.finalUrl || item.url).pathname))
    } catch {
      e = ''
    }
  }
  if (!e && item.mime) e = MIME_EXT[item.mime.split(';')[0].trim().toLowerCase()] || ''
  return e
}

// shouldCapture decides whether a new browser download goes to dlm.
// Unknown size is accepted only when the type matches.
export function shouldCapture(item, cfg, selfId) {
  if (!cfg.enabled) return false
  if (item.byExtensionId && item.byExtensionId === selfId) return false
  const url = item.finalUrl || item.url || ''
  if (!/^https?:\/\//i.test(url)) return false // blob:, data:, file: stay in the browser
  if (item.state && item.state !== 'in_progress') return false
  const ext = extOf(item)
  const types = (cfg.types || []).map((t) => t.toLowerCase())
  if (!ext || !types.includes(ext)) return false
  const size = item.totalBytes > 0 ? item.totalBytes : item.fileSize > 0 ? item.fileSize : 0
  return size === 0 || size >= (cfg.minSize || 0)
}

export function cookieHeader(cookies) {
  return (cookies || []).map((c) => `${c.name}=${c.value}`).join('; ')
}
