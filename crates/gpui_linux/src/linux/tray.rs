//! Tray items over StatusNotifierItem, via `ksni`.
//!
//! `ksni` runs the item and its `com.canonical.dbusmenu` menu on its own
//! thread. Its callbacks only push events into the channel installed by
//! [`gpui::Platform::set_tray_event_sender`]; tray updates travel the other way
//! through [`ksni::blocking::Handle::update`].

use std::{
    path::Path,
    sync::{Arc, atomic::AtomicUsize, atomic::Ordering},
};

use futures::channel::mpsc::UnboundedSender;
use gpui::{PlatformTray, SharedString, TrayEvent, TrayMenuItem, TrayOptions};
use ksni::blocking::{Handle, TrayMethods as _};
use ksni::menu::{CheckmarkItem, MenuItem, StandardItem};
use parking_lot::Mutex;

/// The channel installed through [`gpui::Platform::set_tray_event_sender`].
pub(crate) type TrayEventSender = Arc<Mutex<Option<UnboundedSender<TrayEvent>>>>;

pub(crate) fn new_event_sender() -> TrayEventSender {
    Arc::new(Mutex::new(None))
}

/// Creates a tray item and starts its StatusNotifierItem service.
pub(crate) fn create(
    options: TrayOptions,
    events: TrayEventSender,
) -> Option<Box<dyn PlatformTray>> {
    let tray = SniTray {
        id: format!("gpui-tray-{}", next_tray_id()),
        icon: options.icon.as_deref().map(icon_pixmap).unwrap_or_default(),
        title: options.title,
        tooltip: options.tooltip,
        menu: Vec::new(),
        events,
    };

    match tray.spawn() {
        Ok(handle) => Some(Box::new(LinuxTray { handle })),
        Err(error) => {
            log::warn!("failed to create the tray item: {error}");
            None
        }
    }
}

fn next_tray_id() -> usize {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(1);
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// Owns a tray item for the application.
struct LinuxTray {
    handle: Handle<SniTray>,
}

impl PlatformTray for LinuxTray {
    fn set_menu(&self, items: Vec<TrayMenuItem>) {
        let _ = self.handle.update(move |tray| tray.menu = items);
    }

    fn set_tooltip(&self, tooltip: Option<SharedString>) {
        let _ = self.handle.update(move |tray| tray.tooltip = tooltip);
    }

    fn set_title(&self, _title: Option<SharedString>) {
        // StatusNotifierItem has no title beside the icon.
    }
}

impl Drop for LinuxTray {
    fn drop(&mut self) {
        // Waits for the service to unregister the item.
        self.handle.shutdown().wait();
    }
}

struct SniTray {
    id: String,
    icon: Vec<ksni::Icon>,
    title: Option<SharedString>,
    tooltip: Option<SharedString>,
    menu: Vec<TrayMenuItem>,
    events: TrayEventSender,
}

impl SniTray {
    fn send(&self, event: TrayEvent) {
        if let Some(sender) = self.events.lock().as_ref() {
            sender.unbounded_send(event).ok();
        }
    }
}

impl ksni::Tray for SniTray {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn title(&self) -> String {
        self.title.as_deref().unwrap_or_default().to_string()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.icon.clone()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: self.title.as_deref().unwrap_or_default().to_string(),
            description: self.tooltip.as_deref().unwrap_or_default().to_string(),
            ..Default::default()
        }
    }

    /// A left click. Clicks open the menu when the item has one, so this only
    /// reports an event for a menu-less item.
    fn activate(&mut self, _x: i32, _y: i32) {
        if self.menu.is_empty() {
            self.send(TrayEvent::Click);
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        self.menu
            .iter()
            .map(|item| {
                let id = item.id.clone();
                let events = self.events.clone();
                let activate = Box::new(move |_tray: &mut Self| {
                    if let Some(sender) = events.lock().as_ref() {
                        sender.unbounded_send(TrayEvent::MenuItem(id.clone())).ok();
                    }
                });
                if item.checked {
                    MenuItem::Checkmark(CheckmarkItem {
                        label: item.label.to_string(),
                        enabled: item.enabled,
                        checked: true,
                        activate,
                        ..Default::default()
                    })
                } else {
                    MenuItem::Standard(StandardItem {
                        label: item.label.to_string(),
                        enabled: item.enabled,
                        activate,
                        ..Default::default()
                    })
                }
            })
            .collect()
    }
}

/// Decodes the icon into the ARGB32 pixmap the item carries: StatusNotifierItem
/// has no portable way to reference a file path.
fn icon_pixmap(path: &Path) -> Vec<ksni::Icon> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            log::warn!("failed to read tray icon {path:?}: {error}");
            return Vec::new();
        }
    };
    let image = match image::load_from_memory(&bytes) {
        Ok(image) => image.to_rgba8(),
        Err(error) => {
            log::warn!("failed to decode tray icon {path:?}: {error}");
            return Vec::new();
        }
    };

    let (width, height) = image.dimensions();
    let mut data = image.into_raw();
    for pixel in data.chunks_exact_mut(4) {
        // RGBA to ARGB32 in network byte order.
        pixel.rotate_right(1);
    }
    vec![ksni::Icon {
        width: width as i32,
        height: height as i32,
        data,
    }]
}
