// SPDX-License-Identifier: MPL-2.0
import {invoke} from '@tauri-apps/api/core';
import type {Snapshot} from './contracts';
export interface ReplyArtifactRequest {sceneId:string;messageId:string;code:string;x:number;y:number}
export type RestoreReplyArtifact=(request:ReplyArtifactRequest)=>Promise<void>;
export async function restoreReplyArtifact(request:ReplyArtifactRequest):Promise<Snapshot>{
  return invoke('restore_message_artifact',{...request});
}
