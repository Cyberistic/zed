//! The dock icon's badge, the one label macOS draws outside the app's windows.
//!
//! The badge belongs to the dock tile, so it only exists while the app has a
//! dock icon: a menu-bar-only app (activation policy `Accessory`) has nothing
//! to draw it on and this becomes a no-op.

use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;
use objc2_foundation::NSString;

/// Sets the dock icon's badge, or clears it with `None`.
pub(crate) fn set_dock_badge(label: Option<&str>) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let label = label.map(NSString::from_str);
    NSApplication::sharedApplication(mtm)
        .dockTile()
        .setBadgeLabel(label.as_deref());
}
