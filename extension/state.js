export const API_VERSION = 1;
export const BRIGHTNESS = 0x10;

export function parseState(json) {
    const state = JSON.parse(json);
    if (state.api_version !== API_VERSION)
        throw new Error(`Unsupported service API ${state.api_version}`);
    if (!Array.isArray(state.monitors))
        throw new Error('Monitor state is missing its monitor list');
    return state;
}
export function brightnessMonitors(state) {
    return state.monitors
        .map(monitor => ({
            ...monitor,
            control: monitor.controls?.find(control => control.code === BRIGHTNESS),
        }))
        .filter(monitor => monitor.control?.writable && monitor.control.maximum > 0);
}

export function combinedBrightness(monitors) {
    if (monitors.length === 0)
        return 0;
    return monitors.reduce((sum, monitor) =>
        sum + monitor.control.current * 100 / monitor.control.maximum, 0) / monitors.length;
}
