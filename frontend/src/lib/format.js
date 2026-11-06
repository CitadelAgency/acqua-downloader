// Desktop commands reject with Error objects or strings; show only the message.
export function errText(e) {
  return String(e?.message ?? e).replace(/^Error:\s*/, '')
}

export function bytes(n) {
  if (n == null || n < 0) return '?'
  const u = ['B', 'KiB', 'MiB', 'GiB', 'TiB']
  let i = 0
  while (n >= 1024 && i < u.length - 1) {
    n /= 1024
    i++
  }
  return (i === 0 ? n : n.toFixed(n < 10 ? 2 : 1)) + ' ' + u[i]
}

export function duration(s) {
  if (s == null || s < 0 || !isFinite(s)) return ''
  s = Math.round(s)
  if (s < 60) return s + ' s'
  const m = Math.floor(s / 60)
  if (m < 60) return m + ' min ' + (s % 60) + ' s'
  const h = Math.floor(m / 60)
  return h + ' h ' + (m % 60) + ' min'
}

// parseRate("2M") -> bytes/s (0 = unlimited, null = invalid)
export function parseRate(s) {
  s = (s || '').trim().toUpperCase()
  if (!s) return 0
  const m = s.match(/^(\d+(?:[.,]\d+)?)\s*([KMG])?I?B?(?:\/S)?$/)
  if (!m) return null
  const v = parseFloat(m[1].replace(',', '.'))
  const mult = { K: 1024, M: 1024 ** 2, G: 1024 ** 3 }[m[2]] || 1
  return Math.round(v * mult)
}

export function rateText(bps) {
  return bps > 0 ? bytes(bps).replace('i', '').replace(' ', '') : ''
}
