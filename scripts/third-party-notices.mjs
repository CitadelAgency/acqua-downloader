// Writes THIRD-PARTY-NOTICES.txt from what ships in the app. --check fails if it is stale.
import { execFileSync } from 'node:child_process'
import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'

const root = join(import.meta.dirname, '..')
const output = join(root, 'THIRD-PARTY-NOTICES.txt')
const targets = ['x86_64-pc-windows-msvc', 'aarch64-apple-darwin', 'x86_64-apple-darwin']
const licenseFile = /^(licen[cs]e|copying|copyright|notice|unlicense)/i
const normalize = (text) => text.replace(/\r\n/g, '\n').trim()
const byName = (a, b) => (a < b ? -1 : a > b ? 1 : 0)
const standard = ({ authors, license }) => {
  const holder = `Copyright (c) ${authors}`
  const note = 'This package ships no license file. Standard text of its declared license.'
  const text = /\bMIT\b/.test(license) ? mit : license === 'BSD-3-Clause' ? bsd3 : null
  return text && [note, holder, text].join('\n\n')
}
const mit = `Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.`
const bsd3 = `Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

3. Neither the name of the copyright holder nor the names of its contributors
   may be used to endorse or promote products derived from this software
   without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND
ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.`

function licenseTexts(dir, declared) {
  const files = readdirSync(dir).flatMap((name) => {
    const path = join(dir, name)
    if (statSync(path).isDirectory()) {
      return /^licen[cs]es$/i.test(name) ? readdirSync(path).map((file) => join(path, file)) : []
    }
    return licenseFile.test(name) ? [path] : []
  })
  if (declared) files.push(join(dir, declared))
  return [...new Set(files)].sort(byName).map((file) => normalize(readFileSync(file, 'utf8')))
}

function rustPackages() {
  const found = new Map()
  for (const target of targets) {
    const { packages, resolve } = JSON.parse(execFileSync(
      'cargo',
      ['metadata', '--format-version', '1', '--locked', '--filter-platform', target],
      { cwd: root, encoding: 'utf8', maxBuffer: 1 << 28 },
    ))
    const info = new Map(packages.map((item) => [item.id, item]))
    const nodes = new Map(resolve.nodes.map((node) => [node.id, node]))
    const pending = [resolve.root]
    const seen = new Set(pending)
    while (pending.length) {
      for (const dep of nodes.get(pending.pop()).deps) {
        if (seen.has(dep.pkg) || dep.dep_kinds.every(({ kind }) => kind === 'dev')) continue
        seen.add(dep.pkg)
        pending.push(dep.pkg)
        const item = info.get(dep.pkg)
        found.set(`${item.name} ${item.version}`, {
          label: `${item.name} ${item.version}`,
          license: item.license ?? 'see license text',
          authors: item.authors.join(', ') || `the ${item.name} authors`,
          source: item.repository ?? `https://crates.io/crates/${item.name}`,
          texts: licenseTexts(dirname(item.manifest_path), item.license_file),
        })
      }
    }
  }
  return [...found.values()]
}

// Vite bundles the Svelte runtime. Every other npm package is a build tool.
function npmPackages() {
  const found = new Map()
  const pending = ['svelte']
  while (pending.length) {
    const name = pending.pop()
    if (found.has(name)) continue
    const dir = join(root, 'frontend/node_modules', name)
    const item = JSON.parse(readFileSync(join(dir, 'package.json'), 'utf8'))
    found.set(name, {
      label: `${name} ${item.version} (npm)`,
      license: item.license ?? 'see license text',
      authors: item.author?.name ?? item.author ?? `the ${name} authors`,
      source: `https://www.npmjs.com/package/${name}`,
      texts: licenseTexts(dir),
    })
    pending.push(...Object.keys(item.dependencies ?? {}))
  }
  return [...found.values()]
}

const packages = [...rustPackages(), ...npmPackages()].sort((a, b) => byName(a.label, b.label))
for (const item of packages.filter(({ texts }) => !texts.length)) {
  const shared = packages.find(({ license, texts }) => license === item.license && texts.length)
  item.texts = /MPL/.test(item.license) && shared ? shared.texts : [standard(item)].filter(Boolean)
}
const missing = packages.filter((item) => !item.texts.length).map((item) => `${item.label}: ${item.license}`)
if (missing.length) {
  console.error(`No license file found for:\n${missing.join('\n')}`)
  process.exit(1)
}

const groups = new Map()
for (const item of packages) {
  for (const text of item.texts) groups.set(text, [...(groups.get(text) ?? []), item.label])
}
const rule = '='.repeat(78)
const notices = [
  'Acqua Downloader includes the third-party software listed below. Each package',
  'keeps its own license. The license of Acqua Downloader itself is in LICENSE.',
  '',
  'Packages under the MPL-2.0 are used unmodified. Their source code is available',
  'at the address listed for each one.',
  '',
  rule,
  'Packages',
  rule,
  ...packages.map((item) => `${item.label} | ${item.license} | ${item.source}`),
  '',
  rule,
  'License texts',
  rule,
  ...[...groups].flatMap(([text, labels]) => ['', `Applies to: ${labels.join(', ')}`, '', text, '', '-'.repeat(78)]),
  '',
].join('\n')

if (process.argv.includes('--check')) {
  const current = existsSync(output) ? normalize(readFileSync(output, 'utf8')) : ''
  if (current !== normalize(notices)) {
    console.error('THIRD-PARTY-NOTICES.txt is stale. Run: node scripts/third-party-notices.mjs')
    process.exit(1)
  }
} else {
  writeFileSync(output, notices)
}
console.log(`${packages.length} packages, ${groups.size} license texts`)
