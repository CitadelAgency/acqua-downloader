// Desktop commands are implemented by Rust; the background process owns downloads.
const call = (method, ...args) => window.__TAURI__.core.invoke('rpc', { method, args })
export const List = () => call('List')
export const Add = (request) => call('Add', request)
export const AddText = (text) => call('AddText', text)
export const Pause = (id) => call('Pause', id)
export const Resume = (id) => call('Resume', id)
export const Restart = (id) => call('Restart', id)
export const Remove = (id, partial) => call('Remove', id, partial)
export const PauseAll = () => call('PauseAll')
export const ResumeAll = () => call('ResumeAll')
export const SetPriority = (id, priority) => call('SetPriority', id, priority)
export const SetLimit = (id, bps) => call('SetLimit', id, bps)
export const SetHash = (id, hash) => call('SetHash', id, hash)
export const Relink = (id, url) => call('Relink', id, url)
export const AddMirror = (id, url) => call('AddMirror', id, url)
export const RetryBrowser = (id) => call('RetryBrowser', id)
export const ShowInFolder = (id) => call('ShowInFolder', id)
export const GetSettings = () => call('GetSettings')
export const SaveSettings = (settings) => call('SaveSettings', settings)
export const ChooseDir = () => call('ChooseDir')
export const GetInfo = () => call('GetInfo')
export const ChromeList = () => call('ChromeList')
export const ChromeImport = (items) => call('ChromeImport', items)
export function EventsOn(name, callback) {
  let disposed = false
  let unlisten
  window.__TAURI__.event.listen(name, ({ payload }) => callback(payload)).then((off) => {
    if (disposed) off()
    else unlisten = off
  })
  return () => { disposed = true; unlisten?.() }
}
