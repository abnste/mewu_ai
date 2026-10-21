// SPDX-License-Identifier: MPL-2.0
// This embedded module is also the published @mewu/sdk runtime source.
export const hostApi = 2;
export function definePlugin(activate) { return activate; }

export function createContext(send) {
  const services = new Map();
  const pending = new Map();
  const disposables = [];
  let closed = false;
  let registrationOpen = true;
  let outcome = null;
  let deactivationDone = false;
  // Exception names are untrusted too. Never serialize message, stack, cause,
  // a custom name, or an object coerced to a string across the worker boundary.
  const failureKind = error => {
    try {
      switch (error && error.name) {
        case 'SyntaxError': return 'syntax';
        case 'TypeError': return 'type';
        case 'RangeError': return 'range';
        case 'ReferenceError': return 'reference';
        case 'URIError': return 'uri';
        case 'EvalError': return 'eval';
        case 'Error': return 'error';
        default: return 'unknown';
      }
    } catch { return 'unknown'; }
  };
  const host = Object.freeze({
    call(method, params = {}) {
      if (closed) return Promise.reject(new Error('Extension scope is closed'));
      return new Promise((resolve, reject) => {
        // send cannot receive a host response until this JS turn completes.
        const id = send(method, JSON.stringify(params));
        pending.set(id, { resolve, reject });
      });
    }
  });
  const register = kind => (descriptor, handler) => {
    if (closed || !registrationOpen) throw new Error('Registration scope is closed');
    if (typeof handler !== 'function' || !descriptor || typeof descriptor !== 'object') throw new Error('Invalid service');
    if (services.size >= 128 || services.has(descriptor.id)) throw new Error('Duplicate service or service limit reached');
    const value = JSON.parse(JSON.stringify({ ...descriptor, kind }));
    services.set(value.id, { descriptor: value, handler });
    let disposed = false;
    const dispose = () => { if (!disposed) { disposed = true; services.delete(value.id); } };
    disposables.push(dispose);
    return dispose;
  };
  const storage = Object.freeze({
    get: key => host.call('storage.get', { key }),
    set: (key, value) => host.call('storage.set', { key, value }),
    compareAndSet: (key, expected, value) => host.call('storage.compareAndSet', { key, expected, value }),
    delete: key => host.call('storage.delete', { key }),
    list: ({ cursor = '', prefix = '' } = {}) => host.call('storage.list', { cursor, prefix })
  });
  const ctx = Object.freeze({
    hostApi, host, storage,
    runtime: Object.freeze({ describe: () => host.call('runtime.describe') }),
    configuration: Object.freeze({
      get: key => host.call('configuration.get', { key }),
      compareAndSet: (key, expected, value) => host.call('configuration.compareAndSet', { key, expected, value })
    }),
    commands: Object.freeze({ register: register('command') }),
    tools: Object.freeze({ register: register('tool') }),
    agents: Object.freeze({ register: register('agent') }),
    memory: Object.freeze({ register: register('memory') }),
    models: Object.freeze({ register: register('model') }),
    channels: Object.freeze({ register: register('channel') }),
    renderers: Object.freeze({ register: register('renderer') }),
    ui: Object.freeze({ register: register('ui'), render: value => host.call('ui.render', value) }),
    objects: Object.freeze({
      describe: () => host.call('objects.library.describe'),
      get: () => host.call('objects.library.get'),
      preview: (expectedRevision, id) => host.call('objects.library.preview', {expectedRevision, id}),
      commit: transaction => host.call('objects.library.commit', transaction),
      restore: (expectedRevision, id, position) => host.call('objects.library.restore', { expectedRevision, id, ...position })
    }),
    model: Object.freeze({ invoke: request => host.call('model.invoke', request) }),
    processes: Object.freeze({
      resolve: (candidates, files = []) => host.call('process.resolve', { candidates, files }),
      open: command => host.call('process.open', { command }),
      write: (id, message, bindings) => host.call('process.write', { id, message, ...(bindings ? { bindings } : {}) }),
      read: (id, timeoutMs = 250) => host.call('process.read', { id, timeoutMs }),
      close: id => host.call('process.close', { id })
    }),
    region: Object.freeze({
      observe: regionId => host.call('region.observe', { regionId }),
      action: (regionId, action) => host.call('region.action', { regionId, action })
    }),
    services: Object.freeze({
      register: register('service'),
      list: kind => host.call('services.list', { kind }),
      call: (serviceId, input) => host.call('services.call', { serviceId, input })
    }),
    events: Object.freeze({ emit: (name, data) => host.call('events.emit', { name, data }) }),
    onDispose(callback) {
      if (closed || typeof callback !== 'function') throw new Error('Invalid disposal callback');
      disposables.push(callback);
      return callback;
    }
  });
  return Object.freeze({
    ctx,
    finishRegistration() {
      registrationOpen = false;
      return JSON.stringify([...services.values()].map(v => v.descriptor));
    },
    invoke(serviceId, inputJson, contextJson) {
      if (closed || outcome !== null) throw new Error('Invalid invocation lifecycle');
      const service = services.get(serviceId);
      if (!service) throw new Error('Unknown service');
      outcome = { pending: true };
      Promise.resolve().then(() => service.handler(JSON.parse(inputJson), Object.freeze({ context: JSON.parse(contextJson), host, storage })))
        .then(result => {
          // JSON is the only boundary. Non-JSON output is a diagnosed failure.
          const serialized = JSON.stringify({ ok: true, result: result === undefined ? null : result });
          if (serialized === undefined) throw new Error('Non JSON result');
          outcome = { serialized };
        }).catch(error => { outcome = { serialized: JSON.stringify({ ok: false, failureKind: failureKind(error) }) }; });
    },
    takeOutcome() {
      if (!outcome || outcome.pending) return null;
      const result = outcome.serialized;
      outcome = null;
      return result;
    },
    settle(id, valueJson, rejected) {
      const request = pending.get(id);
      if (!request) throw new Error('Unknown host response');
      pending.delete(id);
      if (rejected) {
        const error = new Error('Host call failed');
        let code;
        try { code = JSON.parse(valueJson)?.code; } catch { /* Legacy host. */ }
        error.code = typeof code === 'string' && /^[A-Za-z0-9_]{1,64}$/.test(code) ? code : 'HOST_ERROR';
        request.reject(error);
      }
      else request.resolve(JSON.parse(valueJson));
    },
    async deactivate() {
      if (closed) return;
      closed = true;
      registrationOpen = false;
      services.clear();
      for (const request of pending.values()) request.reject(new Error('Extension scope is closed'));
      pending.clear();
      for (const dispose of disposables.reverse()) { try { await dispose(); } catch {} }
      deactivationDone = true;
    },
    deactivationDone: () => deactivationDone
  });
}
