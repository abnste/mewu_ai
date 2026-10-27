// SPDX-License-Identifier: MPL-2.0
export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonValue[] | { [key: string]: JsonValue };
export type JsonObject = { [key: string]: JsonValue };
export type Dispose = () => void;
export type ServiceKind = 'command' | 'tool' | 'agent' | 'memory' | 'model' | 'channel' | 'renderer' | 'ui' | 'trigger' | 'service';
export type PluginPlatform = 'any' | 'windows-x86_64' | 'darwin-aarch64' | 'linux-x86_64';
/** SVG paths in a 24×24 viewBox. No markup, URLs, scripts or external assets. */
export interface ExtensionIcon { paths: string[]; color?: string; filled?: boolean }
export interface ExtensionManifest {
  schemaVersion: 2;
  id: string;
  version: string;
  name: string;
  description: string;
  icon?: ExtensionIcon;
  publisher?: string;
  license: string;
  hostApi: 2;
  runtime: 'javascript';
  entry: string;
  /** Omit for one portable JavaScript package shared by all three platforms. */
  platforms?: PluginPlatform[];
  permissions: string[];
  tags: string[];
  dependencies?: Record<string, string>;
}
export interface ServiceDescriptor {
  id: string;
  title: string;
  description?: string;
  inputSchema?: JsonObject;
  /** platforms narrows this feature only; requiresRegion requests a native region grant. */
  metadata?: JsonObject & { platforms?: PluginPlatform[]; requiresRegion?: boolean; requiresMemory?:boolean; requiresProvider?:'agent'|'memory'; modelsService?:string };
}
export interface ServiceRegistration extends ServiceDescriptor { kind: ServiceKind }
/** The host sets context; extension input cannot grant region or agent access. */
export interface InvocationContext {
  /** Host-attested automatic retention; tool arguments cannot set this field. */
  readonly memoryOrigin?: 'completedTurn' | null;
  readonly sceneId?: string;
  readonly agentId?: string;
  readonly runId?: string;
  readonly regionId?: string;
  /** Public conversation messages in this request window, excluding host system
   * messages. Older messages remain saved; this is not a memory retention cap. */
  readonly history?: {
    readonly totalMessages: number;
    readonly includedMessages: number;
    readonly omittedMessages: number;
    readonly truncated: boolean;
  };
  readonly [key: string]: JsonValue | undefined;
}
export interface HostBridge { call<T = JsonValue>(method: string, params?: JsonObject): Promise<T> }
export interface StoragePage { keys: string[]; nextCursor: string | null }
export interface ExtensionStorage {
  get<T extends JsonValue = JsonValue>(key: string): Promise<T | null>;
  set(key: string, value: JsonValue): Promise<JsonValue>;
  /** Atomic CAS. expected=null means the key must be absent, not a stored null. */
  compareAndSet(key: string, expected: JsonValue, value: JsonValue): Promise<boolean>;
  delete(key: string): Promise<JsonValue>;
  /** Prefix filtering and the opaque cursor are distinct. */
  list(options?: { cursor?: string; prefix?: string }): Promise<StoragePage>;
}
export interface HandlerInvocation {
  readonly context: InvocationContext;
  readonly host: HostBridge;
  readonly storage: ExtensionStorage;
}
export type ServiceHandler<Input = JsonValue, Output = JsonValue> =
  (input: Input, invocation: HandlerInvocation) => Output | Promise<Output>;
export interface ServiceRegistry {
  register<Input = JsonValue, Output = JsonValue>(descriptor: ServiceDescriptor, handler: ServiceHandler<Input, Output>): Dispose;
}
export interface RegionPoint { x: number; y: number }
export type RegionKey = 'enter' | 'backspace' | 'delete' | 'arrowLeft' | 'arrowRight' | 'arrowUp' | 'arrowDown' | 'home' | 'end';
export type RegionAction =
  | ({ kind: 'click' | 'doubleClick' } & RegionPoint)
  | { kind: 'drag'; path: RegionPoint[] }
  | ({ kind: 'scroll'; deltaY: number } & RegionPoint)
  | { kind: 'type'; text: string }
  | { kind: 'key'; key: RegionKey };
/** PNG bytes remain in the host artifact store, not the extension JSON heap. */
export interface RegionObservation { artifactId: string; width: number; height: number; sha256: string }
export interface ModelMessage { role: 'user' | 'assistant' | 'system'; content: string }
export interface ModelRequest { prompt: string; images?: string[]; messages?: ModelMessage[] }
export interface ModelReply { content: string; toolCalls: JsonValue[] }
/** Host UI actions a plugin can request for a live capture region. */
export type HostActionName = 'copy' | 'save' | 'pin' | 'reference';
export interface HostActionRequest {
  action: HostActionName;
  /** A regionId from the invocation context, e.g. a selection-toolbar command. */
  regionId: string;
  /** annotated (default) or original pixels; copy and save only. */
  variant?: 'annotated' | 'original';
}
export type HostActionResult = { ok: true } | { ok: false; reason: string };
/** Hold triggers admit modifier keys only; the runtime rejects letters and
 * digits at registration so a trigger can never forward typed characters. */
export interface TriggerDescriptor {
  id: string;
  /** KeyboardEvent name of the modifier key that starts the hold. */
  code: 'Alt' | 'Control' | 'Shift';
  /** Extra modifiers that must be held together with code. */
  with?: ('Ctrl' | 'Shift' | 'Alt')[];
}
export interface TriggerEvent {
  phase: 'down' | 'up' | 'cancel';
  reason?: 'escape' | 'blur' | 'modifiers_changed' | 'scene_changed';
  /** Space-window CSS pixels at the moment of the event. */
  pointer: { x: number; y: number; buttons: number };
  modifiers: { ctrl: boolean; shift: boolean; alt: boolean };
}
export interface RenderedObject {
  html: string;
  width: number;
  height: number;
}
/** A pointer-following transient overlay. The html is passed inline because the
 * calling worker is blocked in this invocation and cannot run another handler. */
export interface TransientDocument {
  html: string;
  width: number;
  height: number;
}
export interface TransientSurface { surfaceId: string }
export interface ExtensionContext {
  readonly runtime: { describe(): Promise<{ hostApi: 2; capabilities: string[] }> };
  readonly hostApi: 2;
  readonly host: HostBridge;
  readonly storage: ExtensionStorage;
  /** Shared plugin configuration. Writes require a live settings surface. */
  readonly configuration: {
    get(key: string): Promise<JsonValue>;
    compareAndSet(key: string, expected: JsonValue, value: JsonValue): Promise<boolean>;
  };
  /** OS-backed plugin credentials. network.fetch authorization:{key,prefix?}
   * attaches them only to that origin. Reading requires separate secrets.read
   * permission and a live settings surface belonging to this exact plugin;
   * Agent/tool/delegated invocations cannot read stored credentials. */
  readonly secrets: {
    status(key: string): Promise<{revision:number;configured:boolean;origin:string|null}>;
    read(key:string,expectedRevision:number): Promise<string>;
    set(key:string,value:string,origin:string,expectedRevision:number): Promise<{revision:number;configured:boolean;origin:string}>;
  };
  readonly commands: ServiceRegistry;
  /** Historical memory migration. list/page/credential/activate require a live
   * settings surface and declared memory.archive permissions. Runtime lookup
   * and forgetting are restricted to the invocation's memory-enabled Agent.
   * Credentials are copied within the native vault and never returned to JS. */
  readonly memoryArchive: {
    list(cursor?:string): Promise<JsonObject>;
    page(bindingId:string,cursor?:string): Promise<JsonObject>;
    lookup(bindingId:string,documentIds:string[]): Promise<JsonValue[]>;
    credential(bindingId:string,expectedRevision:number,secretRevision:number): Promise<JsonObject>;
    activate(request:{bindingId:string;expectedRevision:number;bindingRevision:number;serviceId:string}): Promise<JsonObject>;
    forget(request:{bindingId:string;evidenceId:string;evidenceRevision:number}): Promise<JsonObject>;
  };
  readonly tools: ServiceRegistry;
  readonly agents: ServiceRegistry;
  readonly memory: ServiceRegistry;
  readonly models: ServiceRegistry;
  readonly channels: ServiceRegistry;
  readonly renderers: ServiceRegistry;
  readonly ui: ServiceRegistry & {
    render(value: RenderedObject): Promise<JsonValue>;
    /** Requires ui.overlay. The overlay follows the pointer, passes pointer
     * events through, and is destroyed on pointerup, Escape or window blur. */
    transient(document: TransientDocument): Promise<TransientSurface>;
  };
  /** Atomic, plugin-owned JSON state and retained native objects. Only a live
   * UI surface has access. captureKey consumes one actual user drop. */
  readonly objects: {
    describe(): Promise<{version: 1; capabilities: string[]}>;
    get(): Promise<{revision: number; state: JsonValue; objects: Record<string, JsonValue>}>;
    preview(expectedRevision:number,id:string): Promise<{image:string|null;document?:{html:string;width:number;height:number}}>;
    commit(value: {expectedRevision:number;state:JsonValue;remove?:string[];captureKey?:string}): Promise<JsonValue>;
    restore(expectedRevision:number,id:string,position:{x:number;y:number}): Promise<JsonValue>;
  };
  readonly model: { invoke(request: ModelRequest): Promise<ModelReply> };
  /** Requires process.rpc. Fixed local executables only, no shell/installer.
   * Processes stop on invocation completion, cancellation or revocation. */
  readonly processes: {
    resolve(candidates: string[], files?: string[], searches?: {root:string; relativePath:string}[]): Promise<{ platform: string; executables: string[]; files: string[] }>;
    open(command: { executable: string; args: string[]; cwd?: string; env?: Record<string,string> }): Promise<{ id: string; cwd: string }>;
    write(id: string, message: JsonObject, bindings?: { frameId: string; pointer: string; encoding: 'base64' | 'dataUrl' }[]): Promise<JsonValue>;
    read(id: string, timeoutMs?: number): Promise<JsonObject | null>;
    close(id: string): Promise<JsonValue>;
  };
  readonly region: {
    observe(regionId: string): Promise<RegionObservation>;
    action(regionId: string, action: RegionAction): Promise<JsonValue>;
  };
  /** Host actions on the current scene's region. Each action name requires its
   * own declared permission: copy→action.clipboard, save→action.export,
   * pin→action.pin, reference→action.reference. */
  readonly actions: { invoke(request: HostActionRequest): Promise<HostActionResult> };
  /** Hold-type modifier triggers. The host owns typing-conflict protection and
   * forwards only phase, pointer state and modifier flags. */
  readonly triggers: {
    register(descriptor: TriggerDescriptor, handler: (event: TriggerEvent) => void): Dispose;
  };
  readonly services: ServiceRegistry & {
    list(kind?: ServiceKind): Promise<Array<{ pluginId: string; revision: number; serviceId: string; kind: ServiceKind; title: string }>>;
    call(serviceId: string, input: JsonValue): Promise<JsonValue>;
  };
  readonly events: { emit(name: string, data: JsonValue): Promise<JsonValue> };
  onDispose(callback: () => void | Promise<void>): () => void | Promise<void>;
}
export type PluginActivation = (ctx: ExtensionContext) => void | Promise<void>;
export const hostApi: 2;
export function definePlugin<T extends PluginActivation>(activate: T): T;
/** Low-level runtime adapter, also useful for isolated extension contract tests. */
export interface ExtensionScope {
  readonly ctx: ExtensionContext;
  finishRegistration(): string;
  invoke(serviceId: string, inputJson: string, contextJson: string): void;
  takeOutcome(): string | null;
  settle(id: string, valueJson: string, rejected: boolean): void;
  deactivate(): Promise<void>;
  deactivationDone(): boolean;
}
export function createContext(send: (method: string, paramsJson: string) => string): ExtensionScope;
