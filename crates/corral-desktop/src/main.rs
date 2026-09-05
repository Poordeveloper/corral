#![forbid(unsafe_code)]

use std::rc::Rc;

use corral_client::{ClientActivationPolicy, EndpointSelection};
use corral_desktop::app;
use corral_desktop::bridge::Bridge;
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

    Application::new().run(move |cx| {
        app::bind_keys(cx);
        cx.activate(true);
        app::open_main_window(bridge, cx);
    });
}
