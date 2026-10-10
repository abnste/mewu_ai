// SPDX-License-Identifier: MPL-2.0
export interface TableDocument { index: number; header: string[]; rows: string[][]; align: ('left' | 'center' | 'right' | null)[] }
export interface TableMessage { sourceHash: string; blocks: ({ kind: 'markdown'; text: string } | { kind: 'table'; tableIndex: number })[]; tables: TableDocument[] }
export type TableFormat = 'table' | 'markdown' | 'csv' | 'tsv' | 'png';
export interface TableTarget { sceneId: string; messageId: string; tableIndex: number }
