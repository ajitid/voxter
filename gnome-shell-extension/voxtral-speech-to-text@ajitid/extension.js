import Cairo from 'gi://cairo';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import St from 'gi://St';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const BUS_NAME = 'com.ajitid.VoxtralSpeechToText.Overlay';
const OBJECT_PATH = '/com/ajitid/VoxtralSpeechToText/Overlay';
const WIDTH = 420;
const HEIGHT = 96;

const IFACE_XML = `<node>
  <interface name="com.ajitid.VoxtralSpeechToText.Overlay1">
    <method name="Ping">
      <arg type="s" name="version" direction="out"/>
    </method>
    <method name="SetOverlay">
      <arg type="s" name="state" direction="in"/>
      <arg type="d" name="level" direction="in"/>
    </method>
  </interface>
</node>`;

export default class VoxtralOverlayExtension extends Extension {
    enable() {
        this._state = 'hidden';
        this._level = 0.0;
        this._animationSourceId = 0;
        this._animationStartedUs = GLib.get_monotonic_time();

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
            affectsInputRegion: false,
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

        if (this._actor) {
            Main.layoutManager.removeChrome(this._actor);
            this._actor.destroy();
            this._actor = null;
        }
    }

    Ping() {
        return '1';
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

    _draw(area) {
        const cr = area.get_context();

        cr.save();
        cr.setOperator(Cairo.Operator.CLEAR);
        cr.paint();
        cr.restore();
        cr.setOperator(Cairo.Operator.OVER);

        if (this._state === 'recording' || this._state === 'recording_latch')
            this._drawRecording(cr, this._state === 'recording_latch');
        else if (this._state === 'transcribing')
            this._drawTranscribing(cr);

        cr.$dispose();
    }

    _drawRecording(cr, latch) {
        const displayEnergy = Math.pow(this._level, 0.72);
        const cx = WIDTH * 0.5;
        const halfChord = Math.max(Math.min(WIDTH * 0.26, WIDTH * 0.42), Math.max(WIDTH * 0.18, 48.0));
        const yBase = HEIGHT - 14.0;
        const sagitta = 1.8 + displayEnergy * (HEIGHT * 0.42);
        const left = cx - halfChord;
        const samples = 56;

        cr.setLineWidth(7.0);
        cr.setLineCap(Cairo.LineCap.ROUND);
        for (let i = 0; i <= samples; i++) {
            const t = i / samples;
            const x = left + t * halfChord * 2.0;
            const parabola = 1.0 - Math.pow(t * 2.0 - 1.0, 2.0);
            const y = yBase - sagitta * parabola;
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

        if (latch)
            this._drawLock(cr, cx + halfChord + 20, yBase - sagitta * 0.46);
    }

    _drawLock(cr, x, y) {
        cr.save();
        cr.setSourceRGBA(0.95, 0.82, 0.35, 0.95);
        cr.setLineWidth(2.6);
        cr.arc(x, y - 2, 7, Math.PI, 0);
        cr.stroke();
        this._roundedRectangle(cr, x - 9, y - 2, 18, 15, 4);
        cr.fill();
        cr.restore();
    }

    _drawTranscribing(cr) {
        const elapsed = (GLib.get_monotonic_time() - this._animationStartedUs) / 1_000_000.0;
        const colors = [
            [0.15, 0.55, 1.0],
            [0.55, 0.85, 1.0],
            [0.95, 0.35, 0.75],
        ];

        for (let i = 0; i < 3; i++) {
            const phase = elapsed * 3.4 + i * 0.65;
            const lift = Math.max(0, Math.sin(phase)) * 18;
            const size = 13 + Math.max(0, Math.sin(phase)) * 7;
            const x = WIDTH / 2 - 42 + i * 42 - size / 2;
            const y = HEIGHT / 2 - size / 2 - lift * 0.35;
            const [r, g, b] = colors[i];
            cr.setSourceRGBA(r, g, b, 0.94);
            this._roundedRectangle(cr, x, y, size, size, 4);
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
