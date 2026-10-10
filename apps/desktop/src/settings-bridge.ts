// SPDX-License-Identifier: MPL-2.0
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { settingsClient } from './settings-client';
export const settingsApi = settingsClient({
  invoke: (command, args) => {
    if (!isTauri()) return Promise.reject(new Error('请在桌面版读取或修改此设置'));
    return invoke(command, args);
  },
  listen: async (name, accept) => {
    if (!isTauri()) throw new Error('请在桌面版读取或修改此设置');
    return listen(name, event => accept(event.payload), { target: 'settings' });
  },
});
