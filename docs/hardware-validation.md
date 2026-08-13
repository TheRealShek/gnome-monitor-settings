# Hardware validation checklist

Run this checklist only after the owner approves monitor writes. Record the monitor identity, connector, initial value, requested value, read-back value, physical result, duration, and any stderr output for each step.

1. Confirm DDC/CI is enabled in the monitor's on-screen display.
2. Run `ddcutil detect --brief` as the desktop user, never with `sudo`.
3. Read brightness twice using the detected bus and confirm both reads agree.
4. Start the service and inspect `GetStateJson` over the user D-Bus.
5. Move brightness by one unit in the application and wait for its read-back.
6. Repeat through the Quick Settings per-monitor slider.
7. If multiple monitors are present, test the combined slider with a one-unit change.
8. Disconnect and reconnect a monitor, rescan, and confirm its stable identity.
9. Suspend and resume, then rescan and perform a read before any write.
10. Test failure handling with DDC/CI disabled in the monitor OSD.

Do not validate monitor power-off, reset, input switching, or manufacturer-specific controls; the service intentionally rejects them.
