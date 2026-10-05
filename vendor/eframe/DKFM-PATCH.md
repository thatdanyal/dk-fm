# eframe 0.31.1, patched for DK.FM

Copied unchanged from crates.io except `src/native/run.rs` (`check_redraw_requests` and
`handle_event_result` split into `apply_event_result`). Look for `DK.FM patch` comments.

**Bug fixed:** when a repaint came due for a hidden window, eframe called `request_redraw()`
and switched the event loop to `ControlFlow::Poll`, then waited for `RedrawRequested`. Windows
never sends `WM_PAINT` to a hidden window, so the loop stayed in `Poll` and used 100% of a CPU
core until the window was shown again. DK.FM hides its window when it goes to the tray, and
eframe always asks for one more frame after the close button, so closing to the tray
triggered it every time (and so did song changes while in the tray).

**Fix:** a hidden window's due frame runs directly from `check_redraw_requests` (the same
thing eframe already does for `RepaintNow` on Windows), and the loop goes back to `Wait`
when nothing is left to draw.

When upgrading eframe, check if upstream fixed this; if not, re-apply the patch to the new
version's `run.rs`, or remove the `[patch.crates-io]` entry from Cargo.toml.
