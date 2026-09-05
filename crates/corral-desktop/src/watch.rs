//! The process beyond its windows: what stays when the main window closes.
//!
//! One `Watch` per process, installed before its first window (tray grill
//! Q14): the bridge to corrald, the 1 Hz poll and the list it fills, the
//! tray's presence, and the one Quit gate every exit path runs (Q8). A
//! window presents this and holds nothing the process needs. Closing a
//! window stops presentation; quitting Corral stops watchfulness; neither
//! terminates managed work.

use std::rc::Rc;
use std::time::{Duration, SystemTime};

use gpui::prelude::*;
use gpui::{AnyWindowHandle, App, Context, Entity, Global, PromptLevel, Subscription, Task};

use crate::app;
use crate::bridge::{BRIDGE_STOPPED, Bridge, Polled, Reply, Unanswered};
use crate::quit::{self, Gate, Warning};
use crate::sessions::SessionList;

/// How often the list asks the daemon what it holds. A client refresh policy
/// and not a wire contract: a push channel is a protocol addition and not
/// PR9's (round 1, #3).
const POLL: Duration = Duration::from_secs(1);

/// The main window's persistent line when the status item failed (grill Q14).
pub const TRAY_UNAVAILABLE: &str = "Menu bar icon unavailable — closing this window quits Corral.";

/// Whether this process has a menu-bar status item: the one fact that
/// licenses staying alive without windows (grill Q1). Decided once per run,
/// before the first window, and never retried within it (Q14).
///
/// No variant here claims an item. The tray mechanism, once its probe seals
/// it, adds the variant that owns one; until then no build is watchful.
#[derive(Debug)]
pub enum TrayPresence {
    /// macOS: the status item was not established. The reason is logged and
    /// the main window carries [`TRAY_UNAVAILABLE`].
    Unavailable(String),
    /// A platform with no tray: a known gap, never a failure (Q2).
    Unsupported,
}

impl TrayPresence {
    /// Whether the process outlives its main window. Only an established
    /// status item may keep it: intent to have one is never watchfulness.
    #[must_use]
    pub fn keeps_process(&self) -> bool {
        match self {
            Self::Unavailable(_) | Self::Unsupported => false,
        }
    }

    /// The line the main window shows for the whole run, if any.
    #[must_use]
    pub fn banner(&self) -> Option<&'static str> {
        match self {
            Self::Unavailable(_) => Some(TRAY_UNAVAILABLE),
            Self::Unsupported => None,
        }
    }
}

/// The process's one Watch, for the paths that hold no handle to it: the
/// Dock's reopen is registered before the app exists.
struct ProcessWatch(Entity<Watch>);

impl Global for ProcessWatch {}

pub struct Watch {
    bridge: Rc<Bridge>,
    list: SessionList,
    presence: TrayPresence,
    /// The main window while one is open: the only selector and Open path.
    main_window: Option<AnyWindowHandle>,
    /// One poll in flight at a time (round 1, #3).
    polling: bool,
    /// A Quit confirmation is up; a request meanwhile waits for its answer.
    quit_pending: bool,
    /// The platform has been asked to quit; nothing is asked or opened after.
    quitting: bool,
    _poll: Task<()>,
    _window_closed: Subscription,
}

impl Watch {
    /// Create the process's Watch, once, before its first window.
    pub fn install(bridge: Rc<Bridge>, presence: TrayPresence, cx: &mut App) -> Entity<Self> {
        let watch = cx.new(|cx| Self::new(bridge, presence, cx));
        cx.set_global(ProcessWatch(watch.clone()));
        watch
    }

    /// The installed Watch.
    pub fn of(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<ProcessWatch>()
            .map(|global| global.0.clone())
    }

    fn new(bridge: Rc<Bridge>, presence: TrayPresence, cx: &mut Context<Self>) -> Self {
        let poll = cx.spawn(async move |this, cx| {
            loop {
                match this.update(cx, |this, _| this.begin_poll()) {
                    Ok(Some(reply)) => {
                        let polled = answer(reply.await);
                        if this
                            .update(cx, |this, cx| this.finish_poll(polled, cx))
                            .is_err()
                        {
                            return;
                        }
                    }
                    Ok(None) => {}
                    Err(_) => return,
                }
                cx.background_executor().timer(POLL).await;
            }
        });
        let weak = cx.entity().downgrade();
        let window_closed = cx.on_window_closed(move |cx| {
            let _ = weak.update(cx, |this, cx| this.window_closed(cx));
        });

        Self {
            bridge,
            list: SessionList::default(),
            presence,
            main_window: None,
            polling: false,
            quit_pending: false,
            quitting: false,
            _poll: poll,
            _window_closed: window_closed,
        }
    }

    pub fn bridge(&self) -> &Bridge {
        &self.bridge
    }

    pub fn list(&self) -> &SessionList {
        &self.list
    }

    pub fn presence(&self) -> &TrayPresence {
        &self.presence
    }

    fn begin_poll(&mut self) -> Option<Reply<Result<Polled, Unanswered>>> {
        if self.polling {
            return None;
        }
        self.polling = true;
        Some(self.bridge.poll())
    }

    fn finish_poll(&mut self, polled: Result<Polled, Unanswered>, cx: &mut Context<Self>) {
        self.polling = false;
        self.list.take(polled, SystemTime::now());
        cx.notify();
    }

    /// Ask now rather than at the next tick: after something the person did,
    /// the list they are looking at should be current (round 1, #3).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(reply) = self.begin_poll() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let polled = answer(reply.await);
            let _ = this.update(cx, |this, cx| this.finish_poll(polled, cx));
        })
        .detach();
    }

    pub fn select(&mut self, session_id: &str, cx: &mut Context<Self>) {
        self.list.select(session_id);
        cx.notify();
    }

    pub fn move_selection(&mut self, by: isize, cx: &mut Context<Self>) {
        self.list.move_selection(by);
        cx.notify();
    }

    /// The one way to a main window (grill Q8): the open one, brought
    /// forward, or a new one. `None` only when none could be opened, which
    /// quits: visible and deterministic rather than a process nothing can
    /// reach. Outside any update of the Watch, because the new window's
    /// first paint reads it.
    pub fn ensure_main_window(watch: &Entity<Self>, cx: &mut App) -> Option<AnyWindowHandle> {
        let (quitting, open) = {
            let this = watch.read(cx);
            (this.quitting, this.main_window)
        };
        if quitting {
            return None;
        }
        if let Some(handle) = open {
            cx.activate(true);
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            return Some(handle);
        }
        match app::open_main_window(watch.clone(), cx) {
            Ok(handle) => {
                watch.update(cx, |this, _| this.main_window = Some(handle));
                Some(handle)
            }
            Err(error) => {
                eprintln!("corral-desktop: the window did not open: {error}");
                watch.update(cx, |this, cx| this.quit(cx));
                None
            }
        }
    }

    /// The one Quit gate (grill Q8, Q11): quit, or warn once per attempt when
    /// sessions Corral started continue or cannot be verified.
    pub fn request_quit(&mut self, cx: &mut Context<Self>) {
        if self.quitting || self.quit_pending {
            return;
        }
        let warning = match quit::gate(&self.list) {
            Gate::Quit => {
                self.quit(cx);
                return;
            }
            Gate::Warn(warning) => warning,
        };
        self.quit_pending = true;
        // After the current update: a request from inside the main window
        // (⌘Q pressed in it) cannot enter that window for a prompt.
        let this = cx.entity();
        cx.defer(move |cx| Self::confirm_quit(&this, warning, cx));
    }

    fn confirm_quit(watch: &Entity<Self>, warning: Warning, cx: &mut App) {
        let asked = Self::ensure_main_window(watch, cx).map(|window| {
            window.update(cx, |_, window, cx| {
                window.prompt(
                    PromptLevel::Warning,
                    &warning.message,
                    Some(warning.detail),
                    &["Quit", "Cancel"],
                    cx,
                )
            })
        });
        let Some(Ok(answer)) = asked else {
            watch.update(cx, |this, _| this.quit_pending = false);
            return;
        };
        let watch = watch.clone();
        cx.spawn(async move |cx| {
            // The first button is Quit; a dismissed prompt is Cancel.
            let confirmed = matches!(answer.await, Ok(0));
            let _ = watch.update(cx, |this, cx| {
                this.quit_pending = false;
                if confirmed {
                    this.quit(cx);
                }
            });
        })
        .detach();
    }

    fn window_closed(&mut self, cx: &mut Context<Self>) {
        let Some(main) = self.main_window else {
            return;
        };
        if cx.windows().contains(&main) {
            return;
        }
        self.main_window = None;
        // Without a status item the main window was the only way to reach
        // the process, so closing it is quitting (PR9 round 2 Q7, which the
        // tray supersedes only once established: grill Q1, Q8). corrald's
        // exit stays its own idle lifecycle's.
        if !self.presence.keeps_process() {
            self.quit(cx);
        }
    }

    fn quit(&mut self, cx: &mut Context<Self>) {
        self.quitting = true;
        cx.quit();
    }
}

fn answer(
    reply: Result<Result<Polled, Unanswered>, futures::channel::oneshot::Canceled>,
) -> Result<Polled, Unanswered> {
    reply.unwrap_or_else(|_| Err(Unanswered::Silent(BRIDGE_STOPPED.to_owned())))
}

#[cfg(test)]
#[path = "watch_tests.rs"]
mod tests;
