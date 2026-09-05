//! The main window: every session on the left, the chosen one on the right.
//!
//! See every session, know what needs you, take control. The list is what the
//! daemon last said, as the process's `Watch` polls it every second; the pane
//! shows the selected session's facts, the actions its state and the
//! daemon's capabilities allow, and — once opened — its terminal. Nothing
//! here decides what a session is; that is `corral_client::presentation`'s,
//! rendered. Nothing here outlives the window: what must, the `Watch` owns.

use std::time::SystemTime;

use corral_client::launch::{Continued, Shown, working_directory};
use corral_client::sessions::short_id;
use gpui::prelude::*;
use gpui::{
    AnyView, AnyWindowHandle, App, Bounds, ClickEvent, Context, ElementId, Entity, FocusHandle,
    KeyBinding, Menu, MenuItem, Render, SharedString, StyleRefinement, Subscription,
    TitlebarOptions, Window, WindowBounds, WindowOptions, actions, div, point, px, size,
};

use crate::actions::{NewSessionForm, Offered, Provider};
use crate::bridge::BRIDGE_STOPPED;
use crate::disclosure::{self, Disclosure};
use crate::sessions::Row;
use crate::terminal::{Host, SessionTerminal};
use crate::text_field::TextField;
use crate::theme;
use crate::watch::Watch;

actions!(
    corral_desktop,
    [
        MoveUp,
        MoveDown,
        OpenSelected,
        NewSession,
        Quit,
        Paste,
        Dismiss,
        Confirm,
        Submit
    ]
);

/// The key map. Contexts keep a key to the surface it belongs to: the list's
/// arrows never reach a terminal, and a terminal's `y` never answers a
/// disclosure.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("up", MoveUp, Some("SessionList")),
        KeyBinding::new("down", MoveDown, Some("SessionList")),
        KeyBinding::new("enter", OpenSelected, Some("SessionList")),
        KeyBinding::new("cmd-n", NewSession, Some("MainWindow")),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-v", Paste, Some("Terminal")),
        KeyBinding::new("cmd-v", Paste, Some("TextField")),
        KeyBinding::new("escape", Dismiss, Some("Overlay")),
        KeyBinding::new("y", Confirm, Some("Disclosure")),
        KeyBinding::new("enter", Submit, Some("NewSessionForm")),
    ]);
}

const LIST_WIDTH: f32 = 340.;

/// What an action that could start a runtime meets while a Quit is decided.
const QUIT_PENDING: &str = "Quit is pending: nothing new starts until it is cancelled.";

enum Overlay {
    None,
    NewSession,
    Disclosure(Disclosure),
}

/// The one open attachment, and the window it may have of its own.
struct Opened {
    session_id: String,
    entity: Entity<SessionTerminal>,
    standalone: Option<AnyWindowHandle>,
}

pub struct MainWindow {
    /// The process this window presents: its list, its bridge, its presence.
    watch: Entity<Watch>,
    list_focus: FocusHandle,
    opened: Option<Opened>,
    /// What the last action produced, shown until the next one.
    notice: Option<String>,
    overlay: Overlay,
    overlay_focus: FocusHandle,
    provider: Provider,
    directory: Entity<TextField>,
    arguments: Entity<TextField>,
    /// An action is in flight; nothing else is offered until it lands.
    busy: bool,
    _watch: Subscription,
    _window_closed: Subscription,
}

/// Open the main window over the process's Watch. `Watch::ensure_main_window`
/// is its one caller, so there is one at a time.
pub fn open_main_window(watch: Entity<Watch>, cx: &mut App) -> Result<AnyWindowHandle, String> {
    let bounds = Bounds {
        origin: point(px(80.), px(80.)),
        size: size(px(1180.), px(760.)),
    };
    cx.open_window(
        WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("Corral".into()),
                ..TitlebarOptions::default()
            }),
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(720.), px(480.))),
            ..WindowOptions::default()
        },
        |window, cx| {
            let closing = watch.clone();
            window.on_window_should_close(cx, move |_, cx| {
                closing.update(cx, |watch, cx| watch.main_window_closing(cx))
            });
            let view = cx.new(|cx| MainWindow::new(watch, cx));
            let focus = view.read(cx).list_focus.clone();
            window.focus(&focus);
            window.activate_window();
            view
        },
    )
    .map(AnyWindowHandle::from)
    .map_err(|error| error.to_string())
}

/// The application menu and the global Quit: ⌘Q and "Quit Corral" run the
/// Watch's one gate with or without a window (tray grill Q8). The Dock's
/// Quit and logout do not pass here: gpui 0.2.2 gives the platform's
/// termination no hook before it is decided (plan D4, known gap).
pub fn bind_quit(watch: Entity<Watch>, cx: &mut App) {
    cx.set_menus(vec![Menu {
        name: "Corral".into(),
        items: vec![MenuItem::action("Quit Corral", Quit)],
    }]);
    cx.on_action(move |_: &Quit, cx| {
        watch.update(cx, |watch, cx| watch.request_quit(cx));
    });
}

impl MainWindow {
    fn new(watch: Entity<Watch>, cx: &mut Context<Self>) -> Self {
        // Every generation the Watch takes is a repaint.
        let observed = cx.observe(&watch, |_, _, cx| cx.notify());
        let weak = cx.entity().downgrade();
        let window_closed = cx.on_window_closed(move |cx| {
            let _ = weak.update(cx, |this, cx| this.window_closed(cx));
        });
        let directory = cx.new(|cx| TextField::new(String::new(), "/absolute/path", cx));
        let arguments = cx.new(|cx| TextField::new(String::new(), "provider arguments", cx));

        Self {
            watch,
            list_focus: cx.focus_handle(),
            opened: None,
            notice: None,
            overlay: Overlay::None,
            overlay_focus: cx.focus_handle(),
            provider: Provider::ClaudeCode,
            directory,
            arguments,
            busy: false,
            _watch: observed,
            _window_closed: window_closed,
        }
    }

    /// Ask now rather than at the next tick: after something the person did,
    /// the list they are looking at should be current (round 1, #3).
    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.watch.update(cx, |watch, cx| watch.refresh(cx));
    }

    /// The terminal's own window closed: the pane shows it again. This
    /// window's own closing is the Watch's rule, not this view's.
    fn window_closed(&mut self, cx: &mut Context<Self>) {
        if let Some(opened) = &mut self.opened
            && let Some(standalone) = opened.standalone
            && !cx.windows().contains(&standalone)
        {
            opened.standalone = None;
            opened
                .entity
                .update(cx, |terminal, cx| terminal.set_host(Host::Embedded, cx));
            cx.notify();
        }
    }

    fn offered(&self, cx: &App) -> Offered {
        let list = self.watch.read(cx).list();
        if list.is_current() {
            Offered::by(list.capabilities())
        } else {
            Offered::default()
        }
    }

    fn opened_for(&self, session_id: &str) -> Option<&Opened> {
        self.opened
            .as_ref()
            .filter(|opened| opened.session_id == session_id)
    }

    // ----- actions -----

    fn move_up(&mut self, _: &MoveUp, _window: &mut Window, cx: &mut Context<Self>) {
        self.watch
            .update(cx, |watch, cx| watch.move_selection(-1, cx));
        self.notice = None;
        cx.notify();
    }

    fn move_down(&mut self, _: &MoveDown, _window: &mut Window, cx: &mut Context<Self>) {
        self.watch
            .update(cx, |watch, cx| watch.move_selection(1, cx));
        self.notice = None;
        cx.notify();
    }

    fn select(&mut self, session_id: &str, cx: &mut Context<Self>) {
        self.watch
            .update(cx, |watch, cx| watch.select(session_id, cx));
        self.notice = None;
        cx.notify();
    }

    fn open_selected(&mut self, _: &OpenSelected, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.watch.read(cx).list().selected().cloned() else {
            return;
        };
        // Refused before the request rather than after it: the row already
        // says why, and its execution state is untouched.
        if let Some(refusal) = row.presentation.refuses_open() {
            self.notice = Some(format!("{refusal}: this session cannot be opened."));
            cx.notify();
            return;
        }
        let session_id = row.session_id.clone();
        self.open(session_id, window, cx);
    }

    /// Attach, and show the terminal. One attachment at a time in PR9: the
    /// one open before is detached, wherever it was shown.
    fn open(&mut self, session_id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || !self.watch.read(cx).list().is_current() {
            return;
        }
        if self.opened_for(&session_id).is_some() {
            self.focus_terminal(window, cx);
            return;
        }
        self.close_terminal(cx);
        self.busy = true;
        self.notice = None;
        cx.notify();
        let reply = self.watch.read(cx).attach(session_id.clone());
        cx.spawn_in(window, async move |this, cx| {
            let attached = reply
                .await
                .unwrap_or_else(|_| Err(BRIDGE_STOPPED.to_owned()));
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match attached {
                    Ok(attached) => {
                        let entity =
                            cx.new(|cx| SessionTerminal::new(session_id.clone(), attached, cx));
                        this.opened = Some(Opened {
                            session_id,
                            entity,
                            standalone: None,
                        });
                        this.focus_terminal(window, cx);
                    }
                    Err(error) => this.notice = Some(error),
                }
                this.refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn focus_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(opened) = &self.opened {
            let focus = opened.entity.read(cx).focus_handle().clone();
            window.focus(&focus);
        }
    }

    fn close_terminal(&mut self, cx: &mut Context<Self>) {
        let Some(opened) = self.opened.take() else {
            return;
        };
        if let Some(standalone) = opened.standalone {
            let _ = standalone.update(cx, |_, window, _| window.remove_window());
        }
        opened.entity.update(cx, |terminal, cx| terminal.detach(cx));
        cx.notify();
    }

    fn detach(&mut self, cx: &mut Context<Self>) {
        self.close_terminal(cx);
        self.notice = Some("Detached. The session keeps running.".to_owned());
        self.refresh(cx);
    }

    fn interrupt(&mut self, cx: &mut Context<Self>) {
        if let Some(opened) = &self.opened {
            opened.entity.update(cx, |terminal, _| terminal.interrupt());
        }
    }

    /// Move the terminal to a window of its own. The entity is the same; the
    /// pane shows a placeholder until that window closes (round 1, #5).
    fn open_in_window(&mut self, cx: &mut Context<Self>) {
        let Some(opened) = &self.opened else {
            return;
        };
        if opened.standalone.is_some() {
            return;
        }
        let entity = opened.entity.clone();
        let title: SharedString = format!("Corral — {}", short_id(&opened.session_id)).into();
        let weak = cx.entity().downgrade();
        // Deferred: this window is mid-update, and a second one is opened
        // from outside it.
        cx.defer(move |cx| {
            let bounds = Bounds {
                origin: point(px(160.), px(160.)),
                size: size(px(900.), px(600.)),
            };
            let host_entity = entity.clone();
            let opened = cx.open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some(title),
                        ..TitlebarOptions::default()
                    }),
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..WindowOptions::default()
                },
                move |window, cx| {
                    let focus = host_entity.read(cx).focus_handle().clone();
                    window.focus(&focus);
                    window.activate_window();
                    cx.new(|_| StandaloneHost {
                        terminal: host_entity,
                    })
                },
            );
            let Ok(handle) = opened else {
                return;
            };
            entity.update(cx, |terminal, cx| terminal.set_host(Host::Standalone, cx));
            let _ = weak.update(cx, |this, cx| {
                if let Some(opened) = &mut this.opened {
                    opened.standalone = Some(handle.into());
                }
                cx.notify();
            });
        });
    }

    fn new_session(&mut self, _: &NewSession, window: &mut Window, cx: &mut Context<Self>) {
        if !self.offered(cx).new_session {
            return;
        }
        let form = NewSessionForm::here(self.provider);
        self.directory
            .update(cx, |field, cx| field.set_text(form.working_directory, cx));
        self.arguments
            .update(cx, |field, cx| field.set_text(String::new(), cx));
        self.overlay = Overlay::NewSession;
        self.notice = None;
        let focus = self.directory.read(cx).focus_handle().clone();
        window.focus(&focus);
        cx.notify();
    }

    fn choose_provider(&mut self, provider: Provider, cx: &mut Context<Self>) {
        self.provider = provider;
        cx.notify();
    }

    fn submit(&mut self, _: &Submit, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.overlay, Overlay::NewSession) || self.busy {
            return;
        }
        let form = NewSessionForm {
            provider: self.provider,
            working_directory: self.directory.read(cx).text().to_owned(),
            arguments: self.arguments.read(cx).text().to_owned(),
        };
        let launch = match form.preflight() {
            Ok(launch) => launch,
            Err(preflight) => {
                self.notice = Some(preflight.to_string());
                cx.notify();
                return;
            }
        };
        let Some(reply) = self
            .watch
            .read(cx)
            .start_session(launch.requested, launch.site)
        else {
            self.notice = Some(QUIT_PENDING.to_owned());
            cx.notify();
            return;
        };
        self.busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let started = reply
                .await
                .unwrap_or_else(|_| Err(BRIDGE_STOPPED.to_owned()));
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match started {
                    Ok(started) => {
                        this.overlay = Overlay::None;
                        this.refresh(cx);
                        this.open(started.session_id, window, cx);
                    }
                    // The daemon's refusal, in its words: an unknown agent,
                    // an argument its grammar rejects.
                    Err(error) => this.notice = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn dismiss(&mut self, _: &Dismiss, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.overlay, Overlay::Disclosure(_)) {
            self.notice = Some("Not continued.".to_owned());
        }
        self.overlay = Overlay::None;
        window.focus(&self.list_focus);
        cx.notify();
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        let Overlay::Disclosure(disclosure) = &self.overlay else {
            return;
        };
        let (session_id, revision) = (disclosure.session_id.clone(), disclosure.revision.clone());
        self.overlay = Overlay::None;
        self.continue_session(session_id, Shown::Accepted(revision), window, cx);
    }

    fn continue_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.watch.read(cx).list().selected().cloned() else {
            return;
        };
        if let Some(refusal) = row.presentation.refuses_continue() {
            self.notice = Some(format!("{refusal}: this session cannot be continued."));
            cx.notify();
            return;
        }
        let session_id = row.session_id.clone();
        self.continue_session(session_id, Shown::NotYet, window, cx);
    }

    /// Continue a session as a new Run: the daemon's preflight first, its
    /// disclosure shown and answered before anything starts (ADR 0016 D5).
    fn continue_session(
        &mut self,
        session_id: String,
        shown: Shown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy || !self.offered(cx).continue_in_corral {
            return;
        }
        // This process's own working directory: client policy, the same the
        // CLI and TUI apply, and the directory the disclosure names.
        let Some(reply) =
            self.watch
                .read(cx)
                .continue_session(session_id.clone(), shown, working_directory())
        else {
            self.notice = Some(QUIT_PENDING.to_owned());
            cx.notify();
            return;
        };
        self.busy = true;
        self.notice = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let continued = reply
                .await
                .unwrap_or_else(|_| Err(BRIDGE_STOPPED.to_owned()));
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match continued {
                    Ok(Continued::Started { started }) => {
                        this.refresh(cx);
                        this.open(started.session_id, window, cx);
                    }
                    Ok(Continued::NeedsDisclosure { text, revision }) => {
                        this.overlay = Overlay::Disclosure(Disclosure {
                            session_id,
                            text,
                            revision,
                        });
                        window.focus(&this.overlay_focus);
                    }
                    Err(error) => this.notice = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn acknowledge_selected(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.watch.read(cx).list().selected().cloned() else {
            return;
        };
        let Some(item) = row.presentation.acknowledgeable() else {
            self.notice = Some("Nothing to acknowledge.".to_owned());
            cx.notify();
            return;
        };
        if self.busy || !self.offered(cx).acknowledge {
            return;
        }
        self.busy = true;
        let reply = self
            .watch
            .read(cx)
            .acknowledge(row.session_id.clone(), item.to_owned());
        cx.spawn(async move |this, cx| {
            let acknowledged = reply
                .await
                .unwrap_or_else(|_| Err(BRIDGE_STOPPED.to_owned()));
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.notice = Some(match acknowledged {
                    Ok(()) => "Acknowledged.".to_owned(),
                    Err(error) => error,
                });
                this.refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    // ----- rendering -----

    fn render_list(&self, now: SystemTime, cx: &mut Context<Self>) -> impl IntoElement {
        let offered = self.offered(cx);
        let watch = self.watch.read(cx);
        let list = watch.list();
        let selected = list.selected().map(|row| row.session_id.clone());
        let mut rows = div()
            .id("rows")
            .flex_1()
            .overflow_y_scroll()
            .flex()
            .flex_col();
        for row in list.rows() {
            let is_selected = selected.as_deref() == Some(row.session_id.as_str());
            rows = rows.child(render_row(row, is_selected, cx));
        }
        if list.unreadable() > 0 {
            rows = rows.child(
                div()
                    .px_3()
                    .py_2()
                    .text_color(theme::muted())
                    .child(format!(
                        "{} more this build cannot render yet.",
                        list.unreadable()
                    )),
            );
        }
        if let Some(line) = list.empty_line() {
            rows = rows.child(div().px_3().py_2().text_color(theme::muted()).child(line));
        }

        div()
            .id("session-list")
            .track_focus(&self.list_focus)
            .key_context("SessionList")
            .on_action(cx.listener(Self::move_up))
            .on_action(cx.listener(Self::move_down))
            .on_action(cx.listener(Self::open_selected))
            .w(px(LIST_WIDTH))
            .h_full()
            .flex()
            .flex_col()
            .bg(theme::panel_bg())
            .border_r_1()
            .border_color(theme::border())
            .child(div().px_3().py_2().text_size(px(15.)).child(list.heading()))
            .when_some(watch.presence().banner(), |this, banner| {
                this.child(div().px_3().py_2().text_color(theme::muted()).child(banner))
            })
            .when_some(list.banner(now), |this, banner| {
                this.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_color(theme::danger())
                        .child(banner),
                )
            })
            .child(rows)
            .child(
                div()
                    .px_3()
                    .py_2()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .border_t_1()
                    .border_color(theme::border())
                    .when(offered.new_session && !self.busy, |this| {
                        this.child(button(
                            "new-session",
                            "New Session",
                            cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.new_session(&NewSession, window, cx);
                            }),
                        ))
                    })
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme::muted())
                            .child("↑/↓ move · enter open · ⌘N new · ⌘Q quit"),
                    ),
            )
    }

    fn render_session(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let pane = div()
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .p_3()
            .gap_2()
            .overflow_hidden();
        let list = self.watch.read(cx).list();
        let Some(row) = list.selected() else {
            return pane.child(div().text_color(theme::muted()).child("Select a session."));
        };
        let offered = self.offered(cx);
        let opened = self.opened_for(&row.session_id);
        let attached = opened.is_some_and(|opened| opened.entity.read(cx).is_attached());
        let standalone = opened.is_some_and(|opened| opened.standalone.is_some());
        let current = list.is_current() && !self.busy;

        let mut actions = div().flex().flex_row().flex_wrap().gap_2();
        if current && opened.is_none() && row.presentation.refuses_open().is_none() {
            let session_id = row.session_id.clone();
            actions = actions.child(button(
                "open",
                "Open",
                cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.open(session_id.clone(), window, cx);
                }),
            ));
        }
        if attached {
            actions = actions
                .child(button(
                    "interrupt",
                    "Interrupt",
                    cx.listener(|this, _: &ClickEvent, _, cx| this.interrupt(cx)),
                ))
                .child(button(
                    "detach",
                    "Detach",
                    cx.listener(|this, _: &ClickEvent, _, cx| this.detach(cx)),
                ));
            if !standalone {
                actions = actions.child(button(
                    "open-in-window",
                    "Open in window",
                    cx.listener(|this, _: &ClickEvent, _, cx| this.open_in_window(cx)),
                ));
            }
        } else if opened.is_some() {
            actions = actions.child(button(
                "close",
                "Close",
                cx.listener(|this, _: &ClickEvent, _, cx| this.close_terminal(cx)),
            ));
        }
        if current && offered.continue_in_corral && row.presentation.refuses_continue().is_none() {
            actions = actions.child(button(
                "continue",
                "Continue in Corral",
                cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.continue_selected(window, cx);
                }),
            ));
        }
        if current && offered.acknowledge && row.presentation.acknowledgeable().is_some() {
            actions = actions.child(button(
                "acknowledge",
                "Acknowledge",
                cx.listener(|this, _: &ClickEvent, _, cx| this.acknowledge_selected(cx)),
            ));
        }

        let mut facts = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().text_size(px(16.)).child(format!(
                "{}  {}",
                short_id(&row.session_id),
                row.title
            )))
            .child(div().child(row.presentation.state_line()));
        for line in row.presentation.beneath() {
            facts = facts.child(div().text_color(theme::muted()).child(line));
        }

        let terminal: gpui::AnyElement = match opened {
            Some(opened) if opened.standalone.is_some() => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme::muted())
                .child("Shown in its own window.")
                .into_any_element(),
            Some(opened) => {
                let ended = opened.entity.read(cx).ended_because().map(str::to_owned);
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .min_h(px(120.))
                    .child(
                        div().flex_1().min_h(px(80.)).child(
                            AnyView::from(opened.entity.clone())
                                .cached(StyleRefinement::default().size_full()),
                        ),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme::muted())
                            .child(match ended {
                                Some(reason) => format!("{reason}."),
                                None => "Text selection and copy are not available yet; \
                                         paste is."
                                    .to_owned(),
                            }),
                    )
                    .into_any_element()
            }
            None => div().flex_1().into_any_element(),
        };

        pane.child(facts)
            .child(actions)
            .when_some(self.notice.clone(), |this, notice| {
                this.child(div().text_color(theme::danger()).child(notice))
            })
            .when(self.busy, |this| {
                this.child(div().text_color(theme::muted()).child("Asking corrald…"))
            })
            .child(terminal)
    }

    fn render_new_session(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut providers = div().flex().flex_row().gap_2();
        for provider in Provider::ALL {
            let chosen = provider == self.provider;
            providers = providers.child(
                div()
                    .id(ElementId::Name(provider.label().into()))
                    .px_3()
                    .py_1()
                    .rounded(px(4.))
                    .bg(if chosen {
                        theme::selected_bg()
                    } else {
                        theme::button_bg()
                    })
                    .border_1()
                    .border_color(if chosen {
                        theme::accent()
                    } else {
                        theme::border()
                    })
                    .cursor_pointer()
                    .child(provider.label())
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.choose_provider(provider, cx);
                    })),
            );
        }
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::hsla(0., 0., 0., 0.55))
            .child(
                div()
                    .id("new-session-form")
                    .key_context("Overlay NewSessionForm")
                    .on_action(cx.listener(Self::submit))
                    .w(px(560.))
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .rounded(px(8.))
                    .bg(theme::panel_bg())
                    .border_1()
                    .border_color(theme::border())
                    .child(div().text_size(px(15.)).child("New Session"))
                    .child(field("Agent", providers))
                    .child(field("Working directory", self.directory.clone()))
                    .child(field("Arguments", self.arguments.clone()))
                    .when_some(self.notice.clone(), |this, notice| {
                        this.child(div().text_color(theme::danger()).child(notice))
                    })
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_2()
                            .justify_end()
                            .child(button(
                                "form-cancel",
                                "Cancel",
                                cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.dismiss(&Dismiss, window, cx);
                                }),
                            ))
                            .child(button(
                                "form-start",
                                if self.busy { "Starting…" } else { "Start" },
                                cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.submit(&Submit, window, cx);
                                }),
                            )),
                    )
                    .child(div().text_size(px(11.)).text_color(theme::muted()).child(
                        "Arguments are split on spaces and passed to the agent; \
                                 quoting is not interpreted. enter starts · esc cancels",
                    )),
            )
    }
}

impl Render for MainWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = SystemTime::now();
        let overlay: Option<gpui::AnyElement> = match &self.overlay {
            Overlay::None => None,
            Overlay::NewSession => Some(self.render_new_session(cx).into_any_element()),
            Overlay::Disclosure(disclosure) => Some(
                disclosure::render(
                    disclosure,
                    &self.overlay_focus,
                    cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.confirm(&Confirm, window, cx);
                    }),
                    cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.dismiss(&Dismiss, window, cx);
                    }),
                )
                .into_any_element(),
            ),
        };

        div()
            .key_context("MainWindow")
            .on_action(cx.listener(Self::new_session))
            .on_action(cx.listener(Self::dismiss))
            .on_action(cx.listener(Self::confirm))
            .relative()
            .size_full()
            .flex()
            .flex_row()
            .bg(theme::window_bg())
            .text_color(theme::text())
            .text_size(theme::ui_font_px())
            .child(self.render_list(now, cx))
            .child(self.render_session(cx))
            .children(overlay)
    }
}

/// A terminal in a window of its own: the same entity, the whole window.
struct StandaloneHost {
    terminal: Entity<SessionTerminal>,
}

impl Render for StandaloneHost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(theme::terminal_bg()).child(
            AnyView::from(self.terminal.clone()).cached(StyleRefinement::default().size_full()),
        )
    }
}

fn render_row(row: &Row, selected: bool, cx: &Context<MainWindow>) -> impl IntoElement {
    let session_id = row.session_id.clone();
    let mut lines = div().flex().flex_col();
    lines = lines.child(div().child(format!("{}  {}", short_id(&row.session_id), row.title)));
    lines = lines.child(
        div()
            .text_color(theme::muted())
            .child(row.presentation.state_line()),
    );
    for line in row.presentation.beneath() {
        lines = lines.child(div().text_color(theme::muted()).child(line));
    }
    div()
        .id(ElementId::Name(row.session_id.clone().into()))
        .px_3()
        .py_2()
        .border_l_2()
        .border_color(if selected {
            theme::accent()
        } else {
            theme::panel_bg()
        })
        .bg(if selected {
            theme::selected_bg()
        } else {
            theme::panel_bg()
        })
        .hover(|style| style.bg(theme::hover_bg()))
        .cursor_pointer()
        .child(lines)
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.select(&session_id, cx);
        }))
}

fn field(label: &'static str, control: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_color(theme::muted()).child(label))
        .child(control)
}

/// A button: a few pixels of padding around a label, and a click.
pub fn button(
    id: &'static str,
    label: impl Into<SharedString>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .px_3()
        .py_1()
        .rounded(px(4.))
        .bg(theme::button_bg())
        .hover(|style| style.bg(theme::hover_bg()))
        .cursor_pointer()
        .text_color(theme::text())
        .child(label.into())
        .on_click(on_click)
}
