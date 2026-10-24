//! The clipboard on Wayland, where the app lacks the data-control protocol.
//!
//! `arboard` reaches the Wayland clipboard through the privileged data-control protocols
//! (`ext_data_control_v1` / `zwlr_data_control_v1`), which need no window or focus, but
//! which some environments do not allow the app to use. `arboard` is preferred wherever
//! data-control is available, as it gives the unrestricted clipboard access that apps get on
//! other platforms. Once a window is open on Wayland, its connection shows whether the app has
//! data-control: if not, the clipboard uses the core protocol (`wl_data_device`) on that
//! connection, which works while one of the app's windows has keyboard focus.

use std::io::ErrorKind;

use bevy_app::{App, AppExit, Last, PreUpdate};
use bevy_ecs::prelude::*;
use bevy_platform::cell::SyncCell;
use bevy_window::{RawHandleWrapper, Window};
use raw_window_handle::RawDisplayHandle;
use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol::wl_registry::{self, WlRegistry},
    Connection, Dispatch, QueueHandle,
};

use crate::{Clipboard, ClipboardError};

/// The Wayland clipboard of a [`Clipboard`].
pub(crate) enum Wayland {
    /// No window has been opened on Wayland yet: `arboard` is used.
    Unchecked,
    /// The app has data-control, or does not run on Wayland: `arboard` is used.
    Arboard,
    /// The app lacks data-control: the core protocol is used.
    Core(CoreClipboard),
}

/// The core protocol clipboard on the connection of the app's windows.
pub(crate) struct CoreClipboard {
    clipboard: SyncCell<smithay_clipboard::Clipboard>,
    /// Whether one of the app's windows has keyboard focus.
    focused: bool,
}

impl CoreClipboard {
    /// Fails the `operation` with a warning if no window has keyboard focus.
    fn check_focus(&self, operation: &str) -> Result<(), ClipboardError> {
        if self.focused {
            return Ok(());
        }
        bevy_log::warn!(
            "Clipboard {operation} failed: no window has keyboard focus, and access to \
             privileged clipboard protocols is unavailable"
        );
        Err(ClipboardError::Unknown {
            description: "no window has keyboard focus".into(),
        })
    }

    pub(crate) fn load(&mut self) -> Result<String, ClipboardError> {
        self.check_focus("read")?;
        self.clipboard.get().load().map_err(|e| {
            // An empty clipboard, or one without text. `smithay-clipboard` reports an empty one
            // only through its error message.
            if e.kind() == ErrorKind::NotFound || e.to_string() == "selection is empty" {
                return ClipboardError::ContentNotAvailable;
            }
            bevy_log::warn!("Clipboard read failed: {e}");
            ClipboardError::Unknown {
                description: e.to_string(),
            }
        })
    }

    pub(crate) fn store(&mut self, text: String) -> Result<(), ClipboardError> {
        self.check_focus("write")?;
        self.clipboard.get().store(text);
        Ok(())
    }
}

pub(crate) fn build(app: &mut App) {
    app.add_observer(connect)
        .add_systems(PreUpdate, track_focus)
        .add_systems(Last, disconnect);
}

/// Chooses the clipboard once the first window is open.
fn connect(
    add: On<Add<RawHandleWrapper>>,
    windows: Query<&RawHandleWrapper>,
    mut clipboard: ResMut<Clipboard>,
) {
    if !matches!(clipboard.wayland, Wayland::Unchecked) {
        return;
    }
    let Ok(handle) = windows.get(add.entity) else {
        return;
    };
    let RawDisplayHandle::Wayland(display) = handle.get_display_handle() else {
        clipboard.wayland = Wayland::Arboard;
        return;
    };
    let display = display.display.as_ptr();
    if clipboard.system_clipboard.is_some() && has_data_control(display) {
        bevy_log::debug!("Clipboard through data-control");
        clipboard.wayland = Wayland::Arboard;
        return;
    }
    #[expect(
        unsafe_code,
        reason = "the clipboard runs on the window's own `wl_display`, given as a pointer"
    )]
    // SAFETY: the display is the windowing backend's connection, which stays open until its
    // event loop ends. The clipboard is dropped before that: by `disconnect` on `AppExit`, and
    // with the world, which `bevy_winit` clears as its event loop exits.
    let core = unsafe { smithay_clipboard::Clipboard::new(display) };
    bevy_log::debug!(
        "Clipboard through the core protocol: access to privileged clipboard protocols is \
         unavailable"
    );
    clipboard.wayland = Wayland::Core(CoreClipboard {
        clipboard: SyncCell::new(core),
        focused: false,
    });
}

/// Whether the connection has one of the data-control protocols, which `arboard` uses.
fn has_data_control(display: *mut core::ffi::c_void) -> bool {
    struct Globals;
    impl Dispatch<WlRegistry, GlobalListContents> for Globals {
        fn event(
            _: &mut Self,
            _: &WlRegistry,
            _: wl_registry::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    #[expect(
        unsafe_code,
        reason = "the globals are listed on the window's own `wl_display`, given as a pointer"
    )]
    // SAFETY: the display is the windowing backend's open connection; the backend made here
    // neither outlives this function nor closes the connection.
    let backend = unsafe { wayland_backend::client::Backend::from_foreign_display(display.cast()) };
    match registry_queue_init::<Globals>(&Connection::from_backend(backend)) {
        Ok((globals, _)) => globals.contents().with_list(|globals| {
            globals.iter().any(|global| {
                matches!(
                    global.interface.as_str(),
                    "ext_data_control_manager_v1" | "zwlr_data_control_manager_v1"
                )
            })
        }),
        Err(e) => {
            bevy_log::warn!("Listing the Wayland globals failed: {e}");
            false
        }
    }
}

/// Keeps track of whether one of the app's windows has keyboard focus.
fn track_focus(windows: Query<&Window>, mut clipboard: ResMut<Clipboard>) {
    if let Wayland::Core(core) = &mut clipboard.bypass_change_detection().wayland {
        core.focused = windows.iter().any(|window| window.focused);
    }
}

/// Drops the core protocol clipboard as the app exits, while its connection is still open.
fn disconnect(mut exit: MessageReader<AppExit>, mut clipboard: ResMut<Clipboard>) {
    if exit.read().last().is_some() {
        clipboard.wayland = Wayland::Arboard;
    }
}
