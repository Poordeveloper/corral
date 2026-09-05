#![forbid(unsafe_code)]

use std::rc::Rc;

use corral_client::{ClientActivationPolicy, EndpointSelection};
use corral_desktop::app;
use corral_desktop::bridge::Bridge;
use corral_desktop::watch::{TrayPresence, Watch};
use gpui::Application;

fn main() {
    // Resolved once, the way every surface resolves it: the canonical
    // rendezvous, or the endpoint the environment names. The Desktop is
    // another corrald client and never a daemon lifecycle authority (round 2,
    // Q7), so nothing here chooses, discovers, or keeps a daemon.
    let policy = ClientActivationPolicy::resolve();
    let endpoint = match EndpointSelection::from_environment() {
        Ok(endpoint) => endpoint,
        Err(error) => {
            eprintln!("corral-desktop: {error}");
            std::process::exit(2);
        }
    };
    let bridge = Rc::new(Bridge::start(policy, endpoint));
    // Before the first window, so it already knows which lifecycle rule it
    // lives under (tray grill Q14).
    let presence = tray_presence();

    let application = Application::new();
    // The Dock's reopen: the same path as every other way to a window (Q8).
    application.on_reopen(|cx| {
        if let Some(watch) = Watch::of(cx) {
            Watch::ensure_main_window(&watch, cx);
        }
    });
    application.run(move |cx| {
        app::bind_keys(cx);
        let watch = Watch::install(bridge, presence, cx);
        app::bind_quit(watch.clone(), cx);
        cx.activate(true);
        Watch::ensure_main_window(&watch, cx);
    });
}

/// This build establishes no status item. On macOS that is the failure
/// state — logged, and shown in the window for the run (grill Q14); elsewhere
/// it is the known gap (Q2).
fn tray_presence() -> TrayPresence {
    #[cfg(target_os = "macos")]
    {
        let reason = "no status item in this build: the tray mechanism awaits its probe";
        eprintln!("corral-desktop: menu bar icon unavailable: {reason}");
        TrayPresence::Unavailable(reason.to_owned())
    }
    #[cfg(not(target_os = "macos"))]
    {
        TrayPresence::Unsupported
    }
}
