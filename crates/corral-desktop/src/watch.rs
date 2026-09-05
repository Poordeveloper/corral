//! The process beyond its windows: what stays when the main window closes.
//!
//! One `Watch` per process, installed before its first window (tray grill
//! Q14): the bridge to corrald, the 1 Hz poll and the list it fills, the
//! tray's presence and what its status item is shown, and the one Quit gate
//! every exit Corral offers runs (Q8): ⌘Q, the app menu, the tray's Quit,
//! and closing the main window when no status item keeps the process. The
//! platform's own termination — Dock Quit, logout, shutdown — is the
//! recorded exception: gpui 0.2.2 routes `applicationShouldTerminate`
//! nowhere and only reports `will_terminate`, after the decision (plan D4).
//! A window presents this and holds nothing the process needs. Closing a
//! window stops presentation; quitting Corral stops watchfulness; neither
//! terminates managed work.

use std::fmt;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use corral_client::launch::{Continued, LaunchSite, Requested, Shown};
use corral_protocol::method::SessionNewResult;
use futures::StreamExt;
use gpui::prelude::*;
use gpui::{AnyWindowHandle, App, Context, Entity, Global, PromptLevel, Subscription, Task};

use crate::app;
use crate::bridge::{Attached, BRIDGE_STOPPED, Bridge, Polled, Reply, Unanswered};
use crate::quit::{self, Gate, Warning};
use crate::sessions::SessionList;
use crate::tray::{Clicks, StatusItem, TrayAction, TrayProjection};

/// How often the list asks the daemon what it holds. A client refresh policy
/// and not a wire contract: a push channel is a protocol addition and not
/// PR9's (round 1, #3).
const POLL: Duration = Duration::from_secs(1);

/// The main window's persistent line when the status item failed (grill Q14).
pub const TRAY_UNAVAILABLE: &str = "Menu bar icon unavailable — closing this window quits Corral.";

/// Whether this process has a menu-bar status item: the one fact that
/// licenses staying alive without windows (grill Q1). Decided once per run,
/// before the first window, and never retried within it (Q14).
pub enum TrayPresence {
    /// The status item exists and is shown what the Watch projects; the
    /// process outlives its windows.
    Established(Box<dyn StatusItem>),
    /// macOS: the status item was not established. The reason is logged and
    /// the main window carries [`TRAY_UNAVAILABLE`].
    Unavailable(String),
    /// A platform with no tray: a known gap, never a failure (Q2).
    Unsupported,
}

impl fmt::Debug for TrayPresence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Established(_) => f.write_str("Established"),
            Self::Unavailable(reason) => f.debug_tuple("Unavailable").field(reason).finish(),
            Self::Unsupported => f.write_str("Unsupported"),
        }
    }
}

impl TrayPresence {
    /// Whether the process outlives its main window. Only an established
    /// status item may keep it: intent to have one is never watchfulness.
    #[must_use]
    pub fn keeps_process(&self) -> bool {
        match self {
            Self::Established(_) => true,
            Self::Unavailable(_) | Self::Unsupported => false,
        }
    }

    /// The line the main window shows for the whole run, if any.
    #[must_use]
    pub fn banner(&self) -> Option<&'static str> {
        match self {
            Self::Unavailable(_) => Some(TRAY_UNAVAILABLE),
            Self::Established(_) | Self::Unsupported => None,
        }
    }

    fn item_mut(&mut self) -> Option<&mut dyn StatusItem> {
        match self {
            Self::Established(item) => Some(item.as_mut()),
            Self::Unavailable(_) | Self::Unsupported => None,
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
    /// What the status item was last shown, so it is rebuilt only when the
    /// projection changes (grill Q10).
    shown: Option<TrayProjection>,
    /// The main window while one is open: the only selector and Open path.
    main_window: Option<AnyWindowHandle>,
    /// One poll in flight at a time (round 1, #3).
    polling: bool,
    /// A Quit is being decided — its question is out, or its confirmation is
    /// up. Another request waits for that answer, and nothing that could
    /// start a runtime is sent until Cancel.
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

        let mut this = Self {
            bridge,
            list: SessionList::default(),
            presence,
            shown: None,
            main_window: None,
            polling: false,
            quit_pending: false,
            quitting: false,
            _poll: poll,
            _window_closed: window_closed,
        };
        // The item says "asking" from its first moment, never nothing.
        this.publish(SystemTime::now());
        this
    }

    /// Deliver the status item's clicks to the Watch on gpui's foreground
    /// (grill Q3): the platform's handler only forwarded the id, and here it
    /// becomes an action — or a logged nothing, for an id this build has no
    /// word for.
    pub fn bind_tray(watch: &Entity<Self>, mut clicks: Clicks, cx: &mut App) {
        let watch = watch.clone();
        cx.spawn(async move |cx| {
            while let Some(id) = clicks.next().await {
                let _ = cx.update(|cx| match TrayAction::from_menu_id(&id) {
                    Some(action) => Self::act(&watch, action, cx),
                    None => eprintln!("corral-desktop: menu item ignored: {id}"),
                });
            }
        })
        .detach();
    }

    /// What a click on the status item's menu does. Outside any update of
    /// the Watch, because every path may open the main window.
    pub fn act(watch: &Entity<Self>, action: TrayAction, cx: &mut App) {
        match action {
            TrayAction::OpenCorral | TrayAction::More => {
                Self::ensure_main_window(watch, cx);
            }
            TrayAction::NewSession => {
                if let Some(window) = Self::ensure_main_window(watch, cx) {
                    app::new_session_in(window, cx);
                }
            }
            TrayAction::OpenSession(session_id) => Self::open_session(watch, &session_id, cx),
            TrayAction::Quit => watch.update(cx, |this, cx| this.request_quit(cx)),
        }
    }

    /// A row click resolves the session it named against current truth
    /// (grill Q10). Listed: selected, then the window's own Open path with
    /// its refusals. Gone since the menu was built: the window, showing the
    /// list as it is now, and nothing else — never another session.
    fn open_session(watch: &Entity<Self>, session_id: &str, cx: &mut App) {
        let Some(window) = Self::ensure_main_window(watch, cx) else {
            return;
        };
        let listed = watch
            .read(cx)
            .list
            .rows()
            .iter()
            .any(|row| row.session_id == session_id);
        if !listed {
            return;
        }
        watch.update(cx, |this, cx| this.select(session_id, cx));
        app::open_selected_in(window, cx);
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
        self.take(polled, cx);
    }

    /// One answer, or its absence, for the list and the status item alike.
    fn take(&mut self, polled: Result<Polled, Unanswered>, cx: &mut Context<Self>) {
        let now = SystemTime::now();
        self.list.take(polled, now);
        self.publish(now);
        cx.notify();
    }

    /// Show the status item the list as it stands — only when that differs
    /// from what it shows (grill Q10): the 1 Hz poll is never a per-second
    /// rebuild of native menu objects, and an age changes it once a bucket.
    fn publish(&mut self, now: SystemTime) {
        let Some(item) = self.presence.item_mut() else {
            return;
        };
        let projection = TrayProjection::of(&self.list, now);
        if self.shown.as_ref() == Some(&projection) {
            return;
        }
        match item.show(&projection) {
            Ok(()) => self.shown = Some(projection),
            // The item stands with the generation before; the next answer
            // tries again rather than the item vanishing mid-run (Q14).
            Err(error) => eprintln!("corral-desktop: the menu bar item was not updated: {error}"),
        }
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

    pub fn attach(&self, session_id: String) -> Reply<Result<Attached, String>> {
        self.bridge.attach(session_id)
    }

    pub fn acknowledge(
        &self,
        session_id: String,
        attention_item_id: String,
    ) -> Reply<Result<(), String>> {
        self.bridge.acknowledge(session_id, attention_item_id)
    }

    /// Ask the daemon to start a session — unless a Quit is being decided,
    /// when nothing is sent and `None` says so. The bridge answers in order:
    /// a start queued behind the gate's question would begin a runtime the
    /// answer does not carry, and Quit would commit over it. Cancel lifts
    /// the refusal.
    pub fn start_session(
        &self,
        requested: Requested,
        site: LaunchSite,
    ) -> Option<Reply<Result<SessionNewResult, String>>> {
        if self.quit_pending || self.quitting {
            return None;
        }
        Some(self.bridge.start_session(requested, site))
    }

    /// Continue a session as a new Run, under the same rule as a start: a
    /// Continue may end in one.
    pub fn continue_session(
        &self,
        session_id: String,
        shown: Shown,
        working_directory: Option<PathBuf>,
    ) -> Option<Reply<Result<Continued, String>>> {
        if self.quit_pending || self.quitting {
            return None;
        }
        Some(
            self.bridge
                .continue_session(session_id, shown, working_directory),
        )
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
        // Whichever way in, the person asked for Corral: it comes forward,
        // window and all, from behind whatever they were in.
        cx.activate(true);
        if let Some(handle) = open {
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
    ///
    /// Decided from the answer to a question asked now, never from the last
    /// generation: that one may predate a session Corral just started, or a
    /// daemon that has since gone. The bridge answers in order and within a
    /// budget, so the answer is fresh and bounded.
    pub fn request_quit(&mut self, cx: &mut Context<Self>) {
        if self.quitting || self.quit_pending {
            return;
        }
        self.quit_pending = true;
        let reply = self.bridge.poll();
        cx.spawn(async move |this, cx| {
            let polled = answer(reply.await);
            let _ = this.update(cx, |this, cx| this.decide_quit(polled, cx));
        })
        .detach();
    }

    fn decide_quit(&mut self, polled: Result<Polled, Unanswered>, cx: &mut Context<Self>) {
        self.take(polled, cx);
        match quit::gate(&self.list) {
            Gate::Quit => self.quit(cx),
            Gate::Warn(warning) => {
                // After this update: the window opened for the prompt reads
                // the Watch on its first paint.
                let this = cx.entity();
                cx.defer(move |cx| Self::confirm_quit(&this, warning, cx));
            }
        }
    }

    /// Whether the main window may close now. With a status item it simply
    /// closes; without one, closing it is quitting, so the close runs the
    /// gate and the window stays until the gate has answered — a Quit ends
    /// the process, window and all, and a Cancel keeps both.
    pub fn main_window_closing(&mut self, cx: &mut Context<Self>) -> bool {
        if self.quitting || self.presence.keeps_process() {
            return true;
        }
        self.request_quit(cx);
        false
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
        // the process (PR9 round 2 Q7, which the tray supersedes only once
        // established: grill Q1, Q8). The gate ran when the close was
        // requested; a window that went without asking — nothing here removes
        // one — must not leave a process no surface can reach.
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
