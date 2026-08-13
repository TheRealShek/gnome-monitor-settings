import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as QuickSettings from 'resource:///org/gnome/shell/ui/quickSettings.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

import {
    BRIGHTNESS,
    SERVICE_NAME,
    brightnessMonitors,
    combinedBrightness,
    parseState,
} from './state.js';

const OBJECT_PATH = '/io/github/avifenesh/GnomeMonitorSettings';
const APPLICATION_ID = 'io.github.avifenesh.GnomeMonitorSettings.desktop';

const ServiceProxy = Gio.DBusProxy.makeProxyWrapper(`
<node>
  <interface name="io.github.avifenesh.GnomeMonitorSettings1">
    <method name="GetStateJson">
      <arg type="s" direction="out" name="state_json"/>
    </method>
    <method name="Rescan">
      <arg type="s" direction="out" name="state_json"/>
    </method>
    <method name="SetControl">
      <arg type="s" direction="in" name="monitor_id"/>
      <arg type="y" direction="in" name="code"/>
      <arg type="q" direction="in" name="value"/>
      <arg type="s" direction="out" name="state_json"/>
    </method>
    <method name="SetAllBrightness">
      <arg type="q" direction="in" name="value"/>
      <arg type="s" direction="out" name="state_json"/>
    </method>
    <signal name="StateChanged">
      <arg type="s" name="state_json"/>
    </signal>
  </interface>
</node>`);

const BrightnessSlider = GObject.registerClass(
class BrightnessSlider extends QuickSettings.QuickSlider {
    constructor(controller, monitor, allMonitors = false) {
        const title = allMonitors ? 'All external monitors' : monitor.name;
        super({
            iconName: allMonitors ? 'display-brightness-symbolic' : 'video-display-symbolic',
            iconLabel: `${title} brightness`,
        });

        this._controller = controller;
        this._monitorId = allMonitors ? null : monitor.id;
        this._allMonitors = allMonitors;
        this._maximum = allMonitors ? 100 : Math.max(1, monitor.control.maximum);
        this._timeoutId = 0;
        this._sliderChangedId = this.slider.connect('notify::value',
            () => this._queueWrite());
        this.slider.accessible_name = `${title} brightness`;
        this.iconReactive = true;
        this.connect('icon-clicked', () => this._controller.openApplication());
        this.setValue(allMonitors ? monitor.value : monitor.control.current);
    }

    setValue(value) {
        if (this._timeoutId)
            GLib.source_remove(this._timeoutId);
        this._timeoutId = 0;
        this.slider.block_signal_handler(this._sliderChangedId);
        this.slider.value = Math.max(0, Math.min(1, value / this._maximum));
        this.slider.unblock_signal_handler(this._sliderChangedId);
    }

    _queueWrite() {
        if (this._timeoutId)
            GLib.source_remove(this._timeoutId);

        this._timeoutId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 250, () => {
            this._timeoutId = 0;
            const value = Math.round(this.slider.value * this._maximum);
            this._controller.setBrightness(this._monitorId, value, this._allMonitors);
            return GLib.SOURCE_REMOVE;
        });
    }

    destroy() {
        if (this._timeoutId)
            GLib.source_remove(this._timeoutId);
        this._timeoutId = 0;
        super.destroy();
    }
});

const MonitorIndicator = GObject.registerClass(
class MonitorIndicator extends QuickSettings.SystemIndicator {
    constructor(controller, state) {
        super();
        this._sliders = new Map();
        this._indicator = this._addIndicator();
        this._indicator.icon_name = 'display-brightness-symbolic';
        this._indicator.visible = state.monitors.length > 0;

        const monitors = brightnessMonitors(state);

        if (monitors.length > 1) {
            const value = combinedBrightness(monitors);
            const slider = new BrightnessSlider(controller, {value}, true);
            this._sliders.set('all', slider);
            this.quickSettingsItems.push(slider);
        }

        for (const monitor of monitors) {
            const slider = new BrightnessSlider(controller, monitor);
            this._sliders.set(monitor.id, slider);
            this.quickSettingsItems.push(slider);
        }
    }

    update(state) {
        const monitors = brightnessMonitors(state);
        if (monitors.length > 1)
            this._sliders.get('all')?.setValue(combinedBrightness(monitors));
        for (const monitor of monitors)
            this._sliders.get(monitor.id)?.setValue(monitor.control.current);
    }

    destroy() {
        this.quickSettingsItems.forEach(item => item.destroy());
        this.quickSettingsItems = [];
        this._sliders.clear();
        super.destroy();
    }
});

class Controller {
    constructor() {
        this._proxy = null;
        this._indicator = null;
        this._destroyed = false;
        this._signalId = 0;

        this._proxy = new ServiceProxy(Gio.DBus.session, SERVICE_NAME, OBJECT_PATH,
            (proxy, error) => {
                if (this._destroyed)
                    return;
                if (error) {
                    console.error(`Monitor Settings service unavailable: ${error.message}`);
                    return;
                }
                this._signalId = proxy.connectSignal('StateChanged',
                    (_proxy, _sender, [json]) => this._acceptState(json));
                proxy.GetStateJsonRemote((result, callError) => {
                    if (callError) {
                        console.error(`Could not read monitor state: ${callError.message}`);
                        return;
                    }
                    const state = this._acceptState(result[0]);
                    if (state && !state.ready) {
                        proxy.RescanRemote((rescanResult, rescanError) => {
                            if (rescanError) {
                                console.error(`Could not rescan monitors: ${rescanError.message}`);
                                return;
                            }
                            this._acceptState(rescanResult[0]);
                        });
                    }
                });
            });
    }

    _acceptState(json) {
        if (this._destroyed)
            return null;
        try {
            const state = parseState(json);
            this._replaceIndicator(state);
            return state;
        } catch (error) {
            console.error(`Invalid Monitor Settings state: ${error.message}`);
            return null;
        }
    }

    _replaceIndicator(state) {
        const nextLayout = brightnessMonitors(state).map(monitor => monitor.id).sort();
        if (nextLayout.length > 1)
            nextLayout.unshift('all');
        if (this._indicator &&
            JSON.stringify([...this._indicator._sliders.keys()].sort()) ===
                JSON.stringify(nextLayout.sort())) {
            this._indicator.update(state);
            return;
        }
        this._indicator?.destroy();
        this._indicator = new MonitorIndicator(this, state);
        Main.panel.statusArea.quickSettings.addExternalIndicator(this._indicator, 2);
    }

    setBrightness(monitorId, value, allMonitors) {
        const callback = (_result, error) => {
            if (error) {
                Main.notifyError('Monitor brightness failed', error.message);
            }
        };
        if (allMonitors)
            this._proxy.SetAllBrightnessRemote(value, callback);
        else
            this._proxy.SetControlRemote(monitorId, BRIGHTNESS, value, callback);
    }

    openApplication() {
        const appInfo = Gio.DesktopAppInfo.new(APPLICATION_ID);
        if (!appInfo) {
            Main.notifyError('Monitor Settings is not installed',
                'The desktop application could not be found.');
            return;
        }
        appInfo.launch([], null);
    }

    destroy() {
        this._destroyed = true;
        this._indicator?.destroy();
        this._indicator = null;
        if (this._proxy && this._signalId)
            this._proxy.disconnectSignal(this._signalId);
        this._signalId = 0;
        this._proxy = null;
    }
}

export default class MonitorSettingsExtension extends Extension {
    enable() {
        this._controller = new Controller();
    }

    disable() {
        this._controller?.destroy();
        this._controller = null;
    }
}
