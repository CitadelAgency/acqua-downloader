import './style.css'
import { mount } from 'svelte'
import App from './App.svelte'
import { isMac } from './lib/platform.js'

document.documentElement.classList.toggle('mac', isMac)

export default mount(App, { target: document.getElementById('app') })
