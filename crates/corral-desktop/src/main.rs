#![forbid(unsafe_code)]

use std::rc::Rc;

use corral_client::{ClientActivationPolicy, EndpointSelection};
use corral_desktop::app;
use corral_desktop::bridge::Bridge;
use corral_desktop::tray::Clicks;
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

    let application = Application::new();
    // The Dock's reopen: the same path as every other way to a window (Q8).
    application.on_reopen(|cx| {
        if let Some(watch) = Watch::of(cx) {
            Watch::ensure_main_window(&watch, cx);
        }
    });
    application.run(move |cx| {
        app::bind_keys(cx);
        // Inside the running loop, which the status item needs, and before
        // the first window, so the process already knows which lifecycle
        // rule it lives under (tray grill Q14).
        let (presence, clicks) = tray_presence();
        let watch = Watch::install(bridge, presence, cx);
        if let Some(clicks) = clicks {
            Watch::bind_tray(&watch, clicks, cx);
        }
        app::bind_quit(watch.clone(), cx);
        Watch::ensure_main_window(&watch, cx);
    });
}

/// The status item, or why there is none: on macOS a failure to establish
/// it — logged, and shown in the window for the run (grill Q14); elsewhere
/// the known gap (Q2).
fn tray_presence() -> (TrayPresence, Option<Clicks>) {
    #[cfg(target_os = "macos")]
    {
        match corral_desktop::tray::macos::Tray::establish() {
            Ok((tray, clicks)) => (TrayPresence::Established(Box::new(tray)), Some(clicks)),
            Err(reason) => {
                eprintln!("corral-desktop: menu bar icon unavailable: {reason}");
                (TrayPresence::Unavailable(reason), None)
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        (TrayPresence::Unsupported, None)
    }
}
