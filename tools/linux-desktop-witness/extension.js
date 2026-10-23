// SPDX-License-Identifier: MPL-2.0
// GNOME 50 integration: no application content, text input, screenshots, shell
// evaluation, focus changes, or pointer/keyboard injection is exported here.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Clutter from 'gi://Clutter';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import { Extension } from 'resource:///org/gnome/shell/extensions/extension.js';
import { Lease, pointsFromRequest, rectangle } from './policy.mjs';

const path = '/org/gnome/Shell/Extensions/MewuWitness';
const xml = `<node><interface name="dev.mewu.DesktopWitness">
<method name="Arm"><arg type="s" direction="in"/><arg type="s" direction="out"/></method>
<method name="Snapshot"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="out"/></method>
<method name="KeepAlive"><arg type="s" direction="in"/><arg type="s" direction="out"/></method>
<method name="RequestFrame"><arg type="s" direction="in"/><arg type="s" direction="out"/></method>
<method name="Disarm"><arg type="s" direction="in"/></method>
</interface></node>`;

export default class DesktopWitness extends Extension {
  enable() {
    this._epoch = GLib.uuid_string_random();
    this._accelerator = 0;
    this._ownerSignal = 0;
    this._lease = new Lease(() => Math.floor(GLib.get_monotonic_time() / 1000), () => {
      this._releaseKey();
      if (!this._lease.active && this._ownerSignal) {
        Gio.DBus.session.signal_unsubscribe(this._ownerSignal); this._ownerSignal = 0;
      }
    });
    try {
    this._acceleratorSignal = global.display.connect('accelerator-activated', (_display, action) => {
      if (this._accelerator && action === this._accelerator) this._lease.stop();
    });
    this._timer = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 1000, () => { this._lease.expire(); return GLib.SOURCE_CONTINUE; });
    this._object = Gio.DBusExportedObject.wrapJSObject(xml, this);
    this._object.export(Gio.DBus.session, path);
    } catch (error) { this.disable(); throw error; }
  }

  _releaseKey() {
    if (this._accelerator) {
      Main.wm.allowKeybinding(Meta.external_binding_name_for_action(this._accelerator), Shell.ActionMode.NONE);
      global.display.ungrab_accelerator(this._accelerator);
      this._accelerator = 0;
    }
  }

  _reply(invocation, action, signature = '(s)') {
    try { const result = action(invocation.get_sender()); invocation.return_value(new GLib.Variant(signature, signature === '()' ? [] : [JSON.stringify(result)])); }
    catch { invocation.return_dbus_error('dev.mewu.DesktopWitness.Unavailable', 'Desktop witness unavailable'); }
  }

  ArmAsync([token], invocation) {
    this._reply(invocation, sender => {
      // Called once per lease, never while assembling a snapshot. Only the
      // same-user native application may reserve the fixed Escape stop key.
      const result = Gio.DBus.session.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus',
        'org.freedesktop.DBus', 'GetConnectionUnixProcessID', new GLib.Variant('(s)', [sender]),
        new GLib.VariantType('(u)'), Gio.DBusCallFlags.NONE, 1000, null);
      const [pid] = result.deep_unpack();
      const executable = GLib.file_read_link(`/proc/${pid}/exe`);
      if (!Number.isSafeInteger(pid) || pid <= 0 || GLib.path_get_basename(executable) !== 'MewuAI') throw Error('not native host');
      this._lease.arm(sender, token);
      try {
        this._ownerSignal = Gio.DBus.session.signal_subscribe('org.freedesktop.DBus', 'org.freedesktop.DBus',
          'NameOwnerChanged', '/org/freedesktop/DBus', sender, Gio.DBusSignalFlags.NONE,
          (_c, _sender, _path, _interface, _signal, args) => {
            const [name, oldOwner, newOwner] = args.deep_unpack();
            if (oldOwner && !newOwner) this._lease.disconnected(name);
          });
        this._accelerator = global.display.grab_accelerator('Escape', Meta.KeyBindingFlags.NONE);
        if (!this._accelerator) throw Error('stop unavailable');
        Main.wm.allowKeybinding(Meta.external_binding_name_for_action(this._accelerator), Shell.ActionMode.NORMAL);
        return { protocol: 1, epoch: this._epoch };
      } catch (error) { this._lease.clear(); throw error; }
    });
  }

  SnapshotAsync([token, pointsText], invocation) {
    this._reply(invocation, sender => {
      const points = pointsFromRequest(pointsText);
      const lease = this._lease.snapshot(sender, token);
      if (typeof Main.modalCount !== 'number' || typeof Main.sessionMode.isLocked !== 'boolean') throw Error('unknown shell');
      const actors = global.get_window_actors();
      if (actors.length > 256) throw Error('too many windows');
      const windows = global.display.sort_windows_by_stacking(actors.map(actor => actor.meta_window))
        .filter(window => window.showing_on_its_workspace() && !window.minimized)
        .map(window => {
          const actor = window.get_compositor_private();
          if (!actor || !actor.mapped || actor.get_paint_opacity() !== 255) throw Error('window transitioning');
          const title = window.get_title();
          if (title !== null && (typeof title !== 'string' || title.length > 8192)) throw Error('unknown window');
          const labelDigest = title ? GLib.compute_checksum_for_string(GLib.ChecksumType.SHA256, title, -1) : null;
          return { id: window.get_stable_sequence(), pid: window.get_pid(), frame: rectangle(window.get_frame_rect()), buffer: rectangle(window.get_buffer_rect()), overrideRedirect: window.is_override_redirect(), labelDigest };
        });
      const focus = global.display.get_focus_window();
      const monitors = Main.layoutManager.monitors.map(monitor => ({ id: monitor.index, frame: rectangle(monitor) }));
      if (monitors.length < 1 || monitors.length > 32) throw Error('unknown monitors');
      const hit = (x, y) => {
        let actor = global.stage.get_actor_at_pos(Clutter.PickMode.ALL, x, y);
        for (let depth = 0; actor && depth < 32; depth++, actor = actor.get_parent()) {
          if (actor instanceof Meta.WindowActor) return actor.meta_window.get_stable_sequence();
        }
        return null;
      };
      const hits = points.map(([x, y]) => hit(x, y));
      const [pointerX, pointerY, modifiers] = global.get_pointer();
      const blocked = Main.modalCount > 0 || Main.sessionMode.isLocked || Main.overview.visible || global.display.is_grabbed();
      return { protocol: 1, epoch: this._epoch, stopped: lease.stopped, blocked: Boolean(blocked), modifiers,
        pointer: { x: pointerX, y: pointerY, window: hit(pointerX, pointerY) },
        workspace: global.workspace_manager.get_active_workspace_index(), focus: focus?.get_stable_sequence() ?? null, monitors, windows, hits };
    });
  }

  DisarmAsync([token], invocation) { this._reply(invocation, sender => this._lease.disarm(sender, token), '()'); }
  KeepAliveAsync([token], invocation) { this._reply(invocation, sender => this._lease.snapshot(sender, token)); }
  RequestFrameAsync([token], invocation) {
    this._reply(invocation, sender => {
      const lease = this._lease.snapshot(sender, token);
      if (lease.stopped || Main.modalCount !== 0 || Main.sessionMode.isLocked !== false ||
          Main.overview.visible !== false || global.display.is_grabbed()) throw Error('desktop unavailable');
      // Static desktops need not emit another PipeWire frame until damaged.
      // Only schedule a compositor repaint; pixels still come exclusively
      // through the user's Portal stream, with a post-request PTS barrier.
      global.stage.queue_redraw();
      return { protocol: 1, epoch: this._epoch };
    });
  }

  disable() {
    this._object?.unexport(); this._object = null;
    if (this._timer) GLib.source_remove(this._timer);
    if (this._acceleratorSignal) global.display.disconnect(this._acceleratorSignal);
    this._lease?.clear(); this._releaseKey();
    this._timer = this._ownerSignal = this._acceleratorSignal = 0;
  }
}
