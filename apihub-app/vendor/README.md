# Vendored, patched dependencies

## smithay-clipboard 0.7.3 (#277)

- Provenance: crates.io release **0.7.3** (2025-11-02), checksum="71704c03f739f7745053bde45fa203a46c58d25bc5c4efba1d9a60e9dba81226", unpacked by cargo;
  upstream https://github.com/Smithay/smithay-clipboard. Licence MIT: `smithay-clipboard/LICENSE` (unchanged).
- Used through `[patch.crates-io]` in `apihub-app/Cargo.toml` (pulled by eframe 0.32 → egui-winit → arboard/smithay-clipboard).
- Why: `pointer_frame` does `pointer.data::<PointerData>().unwrap()`; when a pointer device appears or
  disappears while the window is open (a Bluetooth mouse that reconnects), a frame of the released
  `wl_pointer` panics the clipboard thread, and every later copy of the window is lost silently.
- Why a copy and not something else (checked 2026-10-03): 0.7.3 is the latest release on crates.io and
  upstream `master` still has the `unwrap()`; eframe 0.32 offers no hook to replace or rebuild its
  clipboard; a second clipboard owned by the application would not work either (`store_selection`
  needs keyboard focus, known only from a `wl_keyboard.enter` that a new connection never gets while
  the window already has focus).
- Manifest: the `[[example]]` and dev-dependencies of the published `Cargo.toml` are removed (not built here).
- Remove this directory and the `[patch.crates-io]` entry when upstream publishes a fix.

Full diff against the published crate (source only):

```diff
--- a/src/state.rs
+++ b/src/state.rs
@@ -123,17 +123,19 @@
             SelectionTarget::Clipboard => {
                 let mgr = self.data_device_manager_state.as_ref()?;
                 self.data_selection_content = contents;
+                let device = seat.data_device.as_ref()?;
                 let source =
                     mgr.create_copy_paste_source(&self.queue_handle, ALLOWED_MIME_TYPES.iter());
-                source.set_selection(seat.data_device.as_ref().unwrap(), seat.latest_serial);
+                source.set_selection(device, seat.latest_serial);
                 self.data_sources.push(source);
             },
             SelectionTarget::Primary => {
                 let mgr = self.primary_selection_manager_state.as_ref()?;
                 self.primary_selection_content = contents;
+                let device = seat.primary_device.as_ref()?;
                 let source =
                     mgr.create_selection_source(&self.queue_handle, ALLOWED_MIME_TYPES.iter());
-                source.set_selection(seat.primary_device.as_ref().unwrap(), seat.latest_serial);
+                source.set_selection(device, seat.latest_serial);
                 self.primary_sources.push(source);
             },
         }
@@ -287,7 +289,9 @@
         seat: WlSeat,
         capability: Capability,
     ) {
-        let seat_state = self.seats.get_mut(&seat.id()).unwrap();
+        let Some(seat_state) = self.seats.get_mut(&seat.id()) else {
+            return;
+        };
 
         match capability {
             Capability::Keyboard => {
@@ -326,7 +330,9 @@
         seat: WlSeat,
         capability: Capability,
     ) {
-        let seat_state = self.seats.get_mut(&seat.id()).unwrap();
+        let Some(seat_state) = self.seats.get_mut(&seat.id()) else {
+            return;
+        };
         match capability {
             Capability::Keyboard => {
                 seat_state.data_device = None;
@@ -362,7 +368,13 @@
         pointer: &WlPointer,
         events: &[PointerEvent],
     ) {
-        let seat = pointer.data::<PointerData>().unwrap().seat();
+        // apple-kb-monitor #277: events still queued for a pointer released
+        // after a capability change carry no data (dead proxy): ignore them
+        // instead of panicking, which killed the clipboard thread for good.
+        let Some(data) = pointer.data::<PointerData>() else {
+            return;
+        };
+        let seat = data.seat();
         let seat_id = seat.id();
         let seat_state = match self.seats.get_mut(&seat_id) {
             Some(seat_state) => seat_state,
```
