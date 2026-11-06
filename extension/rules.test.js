// node --test extension/
import test from 'node:test'
import assert from 'node:assert/strict'
import { DEFAULTS, shouldCapture, extOf, cookieHeader } from './rules.js'

const big = 50 * 1024 * 1024

test('captures large files of listed types', () => {
  assert.equal(shouldCapture({ url: 'https://h/a.iso', totalBytes: big, state: 'in_progress' }, DEFAULTS, 'me'), true)
  assert.equal(shouldCapture({ url: 'https://h/x?id=1', filename: 'C:\\D\\film.MKV', totalBytes: big }, DEFAULTS, 'me'), true)
})

test('skips small files, unknown types, non-http and our own downloads', () => {
  assert.equal(shouldCapture({ url: 'https://h/a.zip', totalBytes: 1000 }, DEFAULTS, 'me'), false)
  assert.equal(shouldCapture({ url: 'https://h/page.html', totalBytes: big }, DEFAULTS, 'me'), false)
  assert.equal(shouldCapture({ url: 'blob:https://h/uuid', filename: 'a.zip', totalBytes: big }, DEFAULTS, 'me'), false)
  assert.equal(shouldCapture({ url: 'https://h/a.zip', totalBytes: big, byExtensionId: 'me' }, DEFAULTS, 'me'), false)
  assert.equal(shouldCapture({ url: 'https://h/a.zip', totalBytes: big }, { ...DEFAULTS, enabled: false }, 'me'), false)
})

test('unknown size is accepted when the type matches', () => {
  assert.equal(shouldCapture({ url: 'https://h/a.zip', totalBytes: 0 }, DEFAULTS, 'me'), true)
})

test('extension from mime when name and url have none', () => {
  assert.equal(extOf({ url: 'https://h/download?id=9', mime: 'application/x-7z-compressed' }), '7z')
  assert.equal(extOf({ url: 'https://h/arquivo%20final.rar' }), 'rar')
})

test('cookie header', () => {
  assert.equal(cookieHeader([{ name: 'a', value: '1' }, { name: 'sid', value: 'x y' }]), 'a=1; sid=x y')
})
