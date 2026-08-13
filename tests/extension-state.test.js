import {
    SERVICE_NAME,
    brightnessMonitors,
    combinedBrightness,
    parseState,
} from '../extension/state.js';

function assertEqual(actual, expected, message) {
    if (actual !== expected)
        throw new Error(`${message}: expected ${expected}, received ${actual}`);
}

const state = parseState(JSON.stringify({
    api_version: 1,
    ready: true,
    monitors: [
        {
            id: 'first',
            controls: [{code: 0x10, current: 25, maximum: 100, writable: true}],
        },
        {
            id: 'second',
            controls: [{code: 0x10, current: 100, maximum: 200, writable: true}],
        },
        {
            id: 'unsupported',
            controls: [{code: 0x12, current: 20, maximum: 100, writable: true}],
        },
    ],
}));

assertEqual(
    SERVICE_NAME === 'io.github.avifenesh.GnomeMonitorSettings',
    false,
    'keeps the service name distinct from the GTK application ID'
);

const monitors = brightnessMonitors(state);
assertEqual(monitors.length, 2, 'filters brightness-capable monitors');
assertEqual(combinedBrightness(monitors), 37.5, 'averages normalized brightness');

let rejected = false;
try {
    parseState('{"api_version":2,"monitors":[]}');
} catch (_error) {
    rejected = true;
}
assertEqual(rejected, true, 'rejects incompatible service APIs');
