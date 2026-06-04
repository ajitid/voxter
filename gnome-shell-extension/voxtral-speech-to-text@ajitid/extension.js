import Cairo from 'gi://cairo';
import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import St from 'gi://St';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';

const BUS_NAME = 'com.ajitid.VoxtralSpeechToText.Overlay';
const OBJECT_PATH = '/com/ajitid/VoxtralSpeechToText/Overlay';
const APP_BUS_NAME = 'com.ajitid.VoxtralSpeechToText.App';
const APP_OBJECT_PATH = '/com/ajitid/VoxtralSpeechToText/App';
const APP_INTERFACE = 'com.ajitid.VoxtralSpeechToText.App1';
const WIDTH = 420;
const HEIGHT = 96;
const STATUS_ICON_WIDTH = 24;
const STATUS_ICON_HEIGHT = 20;
const STATUS_ICON_SCALE = 1.2;

const IFACE_XML = `<node>
  <interface name="com.ajitid.VoxtralSpeechToText.Overlay1">
    <method name="Ping">
      <arg type="s" name="version" direction="out"/>
    </method>
    <method name="SetOverlay">
      <arg type="s" name="state" direction="in"/>
      <arg type="d" name="level" direction="in"/>
    </method>
    <method name="SetTrayState">
      <arg type="b" name="hasLastTranscript" direction="in"/>
    </method>
  </interface>
</node>`;

export default class VoxtralOverlayExtension extends Extension {
    enable() {
        this._state = 'hidden';
        this._level = 0.0;
        this._animationSourceId = 0;
        this._animationStartedUs = GLib.get_monotonic_time();
        this._appAvailable = false;
        this._hasLastTranscript = false;

        this._indicator = new PanelMenu.Button(0.0, 'Voxtral Speech-to-Text', false);
        this._indicatorIcon = new St.DrawingArea({
            style_class: 'voxtral-status-icon',
            reactive: false,
            can_focus: false,
            width: STATUS_ICON_WIDTH,
            height: STATUS_ICON_HEIGHT,
            x_align: Clutter.ActorAlign.CENTER,
            y_align: Clutter.ActorAlign.CENTER,
        });
        this._indicatorIcon.set_size(STATUS_ICON_WIDTH, STATUS_ICON_HEIGHT);
        this._indicatorIcon.connect('repaint', this._drawStatusIcon.bind(this));
        this._indicator.add_child(this._indicatorIcon);

        this._typeItem = new PopupMenu.PopupMenuItem('Type last transcript');
        this._typeItem.connect('activate', () => this._callApp('TypeLastTranscript'));
        this._indicator.menu.addMenuItem(this._typeItem);

        this._indicator.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());

        this._quitItem = new PopupMenu.PopupMenuItem('Quit');
        this._quitItem.connect('activate', () => this._callApp('Quit'));
        this._indicator.menu.addMenuItem(this._quitItem);

        Main.panel.addToStatusArea('voxtral-speech-to-text', this._indicator, 0, 'right');
        this._updateTrayMenu();

        this._appWatchId = Gio.bus_watch_name(
            Gio.BusType.SESSION,
            APP_BUS_NAME,
            Gio.BusNameWatcherFlags.NONE,
            () => {
                this._appAvailable = true;
                this._updateTrayMenu();
            },
            () => {
                this._appAvailable = false;
                this._hasLastTranscript = false;
                this._updateTrayMenu();
            }
        );

        this._actor = new St.DrawingArea({
            style_class: 'voxtral-overlay-container',
            reactive: false,
            can_focus: false,
            visible: false,
            width: WIDTH,
            height: HEIGHT,
        });
        this._actor.set_size(WIDTH, HEIGHT);
        this._actor.connect('repaint', this._draw.bind(this));

        Main.layoutManager.addTopChrome(this._actor, {
            affectsStruts: false,
            trackFullscreen: true,
        });

        this._monitorsChangedId = Main.layoutManager.connect(
            'monitors-changed',
            () => this._reposition()
        );
        this._reposition();

        this._dbus = Gio.DBusExportedObject.wrapJSObject(IFACE_XML, this);
        this._dbus.export(Gio.DBus.session, OBJECT_PATH);
        this._busId = Gio.bus_own_name_on_connection(
            Gio.DBus.session,
            BUS_NAME,
            Gio.BusNameOwnerFlags.REPLACE,
            null,
            null
        );
    }

    disable() {
        this._stopAnimation();

        if (this._busId) {
            Gio.bus_unown_name(this._busId);
            this._busId = 0;
        }

        if (this._dbus) {
            this._dbus.unexport();
            this._dbus = null;
        }

        if (this._monitorsChangedId) {
            Main.layoutManager.disconnect(this._monitorsChangedId);
            this._monitorsChangedId = 0;
        }

        if (this._appWatchId) {
            Gio.bus_unwatch_name(this._appWatchId);
            this._appWatchId = 0;
        }

        if (this._indicator) {
            this._indicator.destroy();
            this._indicator = null;
        }
        this._typeItem = null;
        this._quitItem = null;
        this._indicatorIcon = null;

        if (this._actor) {
            Main.layoutManager.removeChrome(this._actor);
            this._actor.destroy();
            this._actor = null;
        }
    }

    Ping() {
        return '1';
    }

    SetTrayState(hasLastTranscript) {
        this._hasLastTranscript = Boolean(hasLastTranscript);
        this._updateTrayMenu();
    }

    SetOverlay(state, level) {
        const valid = ['hidden', 'recording', 'recording_latch', 'transcribing'];
        if (!valid.includes(state))
            throw new Error(`invalid overlay state: ${state}`);

        this._state = state;
        this._level = Math.max(0.0, Math.min(1.0, Number(level) || 0.0));

        if (state === 'hidden') {
            this._stopAnimation();
            this._actor.hide();
            return;
        }

        this._actor.show();
        this._reposition();

        if (state === 'transcribing')
            this._startAnimation();
        else
            this._stopAnimation();

        this._actor.queue_repaint();
    }

    _updateTrayMenu() {
        if (this._typeItem)
            this._typeItem.setSensitive(this._appAvailable && this._hasLastTranscript);
        if (this._quitItem)
            this._quitItem.setSensitive(this._appAvailable);
    }

    _callApp(method) {
        if (!this._appAvailable)
            return;

        Gio.DBus.session.call(
            APP_BUS_NAME,
            APP_OBJECT_PATH,
            APP_INTERFACE,
            method,
            null,
            null,
            Gio.DBusCallFlags.NONE,
            -1,
            null,
            (_conn, res) => {
                try {
                    Gio.DBus.session.call_finish(res);
                } catch (e) {
                    logError(e, `Voxtral app D-Bus call failed: ${method}`);
                }
            }
        );
    }

    _reposition() {
        if (!this._actor)
            return;
        const monitor = Main.layoutManager.primaryMonitor;
        if (!monitor)
            return;

        const x = Math.round(monitor.x + (monitor.width - WIDTH) / 2);
        const y = Math.round(monitor.y + monitor.height - HEIGHT - 8);
        this._actor.set_position(x, y);
    }

    _startAnimation() {
        if (this._animationSourceId)
            return;

        this._animationStartedUs = GLib.get_monotonic_time();
        this._animationSourceId = GLib.timeout_add(
            GLib.PRIORITY_DEFAULT,
            33,
            () => {
                if (this._actor)
                    this._actor.queue_repaint();
                return GLib.SOURCE_CONTINUE;
            }
        );
    }

    _stopAnimation() {
        if (this._animationSourceId) {
            GLib.Source.remove(this._animationSourceId);
            this._animationSourceId = 0;
        }
    }

    _drawStatusIcon(area) {
        const cr = area.get_context();
        const themeColor = area.get_theme_node().get_foreground_color();
        const [width, height] = area.get_surface_size();
        const x = width / 2.0 - 11.0 * STATUS_ICON_SCALE;
        const y = height / 2.0 - 10.8 * STATUS_ICON_SCALE;

        cr.save();
        cr.setOperator(Cairo.Operator.CLEAR);
        cr.paint();
        cr.restore();
        cr.setOperator(Cairo.Operator.OVER);

        cr.save();
        cr.translate(x, y);
        cr.scale(STATUS_ICON_SCALE, STATUS_ICON_SCALE);
        cr.setLineCap(Cairo.LineCap.ROUND);
        cr.setLineJoin(Cairo.LineJoin.ROUND);

        const setColor = opacity => cr.setSourceRGBA(
            themeColor.red / 255.0,
            themeColor.green / 255.0,
            themeColor.blue / 255.0,
            (themeColor.alpha / 255.0) * opacity
        );

        setColor(1.0);
        cr.setLineWidth(1.6);
        cr.moveTo(4.6, 10.8);
        cr.lineTo(17.4, 10.8);
        cr.stroke();

        cr.moveTo(5.8, 6.1);
        cr.lineTo(4.7, 10.8);
        cr.lineTo(5.8, 15.5);
        cr.moveTo(16.2, 6.1);
        cr.lineTo(17.3, 10.8);
        cr.lineTo(16.2, 15.5);
        cr.stroke();

        setColor(0.568627);
        cr.setLineWidth(1.15);
        cr.moveTo(6.7, 7.0);
        cr.lineTo(5.8, 5.6);
        cr.moveTo(15.3, 7.0);
        cr.lineTo(16.2, 5.6);
        cr.moveTo(6.7, 14.6);
        cr.lineTo(5.8, 16.0);
        cr.moveTo(15.3, 14.6);
        cr.lineTo(16.2, 16.0);
        cr.stroke();

        cr.moveTo(6.0, 6.3);
        cr.lineTo(11.0, 10.8);
        cr.lineTo(16.0, 6.3);
        cr.moveTo(6.0, 15.3);
        cr.lineTo(11.0, 10.8);
        cr.lineTo(16.0, 15.3);
        cr.moveTo(7.8, 6.2);
        cr.lineTo(14.2, 15.4);
        cr.moveTo(14.2, 6.2);
        cr.lineTo(7.8, 15.4);
        cr.stroke();

        cr.restore();
        cr.$dispose();
    }

    _draw(area) {
        const cr = area.get_context();

        cr.save();
        cr.setOperator(Cairo.Operator.CLEAR);
        cr.paint();
        cr.restore();
        cr.setOperator(Cairo.Operator.OVER);

        if (this._state === 'recording' || this._state === 'recording_latch')
            this._drawRecording(cr);
        else if (this._state === 'transcribing')
            this._drawTranscribing(cr);

        cr.$dispose();
    }

    _drawRecording(cr) {
        const displayEnergy = Math.pow(this._level, 0.72);
        const cx = WIDTH * 0.5;
        const halfChord = Math.max(Math.min(WIDTH * 0.26, WIDTH * 0.42), Math.max(WIDTH * 0.18, 48.0));
        const yBase = HEIGHT - 14.0;
        const sagittaRaw = 1.8 + displayEnergy * (HEIGHT * 0.42);
        const s = Math.max(sagittaRaw, 0.5);
        const a = Math.max(halfChord, 1.0);
        const radius = ((a * a) + (s * s)) / (2.0 * s);
        const cy = yBase + (radius - s);
        const left = cx - a;
        const samples = 56;

        cr.setLineWidth(3.8);
        cr.setLineCap(Cairo.LineCap.ROUND);
        for (let i = 0; i <= samples; i++) {
            const t = i / samples;
            const x = left + t * a * 2.0;
            const dx = x - cx;
            const y = cy - Math.sqrt(Math.max(0, radius * radius - dx * dx));
            if (i === 0)
                cr.moveTo(x, y);
            else
                cr.lineTo(x, y);
        }

        const gradient = new Cairo.LinearGradient(left, 0, left + halfChord * 2.0, 0);
        gradient.addColorStopRGBA(0.0, 0.15, 0.55, 1.0, 0.92);
        gradient.addColorStopRGBA(0.5, 0.55, 0.85, 1.0, 1.0);
        gradient.addColorStopRGBA(1.0, 0.95, 0.35, 0.75, 0.92);
        cr.setSource(gradient);
        cr.stroke();
    }

    _drawTranscribing(cr) {
        const elapsed = (GLib.get_monotonic_time() - this._animationStartedUs) / 1_000_000.0;
        const colors = [
            [0.15, 0.55, 1.0],
            [0.35, 0.70, 1.0],
            [0.55, 0.85, 1.0],
            [0.75, 0.60, 0.88],
            [0.95, 0.35, 0.75],
        ];

        for (let i = 0; i < 5; i++) {
            const phase = elapsed * 3.4 + i * 0.55;
            const lift = Math.max(0, Math.sin(phase)) * 14;
            const size = 6 + Math.max(0, Math.sin(phase)) * 3;
            const x = WIDTH / 2 - 50 + i * 24 - size / 2;
            const y = HEIGHT - 14.0 - size / 2 - lift * 0.35;
            const [r, g, b] = colors[i];
            cr.setSourceRGBA(r, g, b, 0.94);
            this._roundedRectangle(cr, x, y, size, size, 3);
            cr.fill();
        }
    }

    _roundedRectangle(cr, x, y, width, height, radius) {
        const r = Math.min(radius, width / 2, height / 2);
        cr.newSubPath();
        cr.arc(x + width - r, y + r, r, -Math.PI / 2, 0);
        cr.arc(x + width - r, y + height - r, r, 0, Math.PI / 2);
        cr.arc(x + r, y + height - r, r, Math.PI / 2, Math.PI);
        cr.arc(x + r, y + r, r, Math.PI, Math.PI * 1.5);
        cr.closePath();
    }
}
