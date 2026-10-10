// SPDX-License-Identifier: MPL-2.0
import { render } from 'solid-js/web';
import type { Component } from 'solid-js';
import { storedLanguage, uiLanguage, updateLanguage } from './i18n';
import { readPreferences } from './interface-preferences';
import './controls.css';

// Retain application context menus without exposing the WebView browser menu.
document.addEventListener('contextmenu', event => event.preventDefault());
updateLanguage(uiLanguage());
const syncButtonLabels = () => { document.documentElement.dataset.buttonLabels = readPreferences().showButtonLabels === false ? 'hide' : 'show'; };
syncButtonLabels();
window.addEventListener('storage', event => {
  if (event.key === 'mewu.interface.v1' || event.key === null) { updateLanguage(storedLanguage()); syncButtonLabels(); }
});

const root = document.getElementById('root');
if (!root) throw new Error('Missing application root');
const surface = new URLSearchParams(location.search).get('surface');
if (surface) document.documentElement.dataset.surface = surface;
if (surface !== 'pin') await Promise.all([import('./styles.css'), import('./space.css')]);
// Each loader retains its own production CSS dependencies. A conditional
// import expression can collapse several windows into one preload list.
const loaders = import.meta.glob<{ default: Component }>([
  './PinView.tsx', './FrozenWidget.tsx', './RecordingSurface.tsx',
  './ScrollSurface.tsx', './SettingsSurface.tsx', './App.tsx',
]);
const paths: Record<string, string> = {
  pin: './PinView.tsx', frozen: './FrozenWidget.tsx', recording: './RecordingSurface.tsx',
  scroll: './ScrollSurface.tsx', settings: './SettingsSurface.tsx',
};
const { default: Surface } = await loaders[paths[surface ?? ''] ?? './App.tsx']();
render(() => <Surface />, root);
