// SPDX-License-Identifier: MPL-2.0
import { register } from 'node:module';
register('./sdk-loader.mjs', import.meta.url);
