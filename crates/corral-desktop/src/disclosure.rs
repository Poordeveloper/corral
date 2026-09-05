//! The continuation disclosure: the daemon's words, shown before anything is
//! started, and answered by the person (ADR 0016 D5).
//!
//! Nothing is summarized or shortened: what the person is agreeing to is
//! exactly what Corral said it would do. Yes is one deliberate act — the
//! button, or `y` — and everything else is no, because the question is about
//! starting another provider process on a conversation that may be in use.

use gpui::{
    App, ClickEvent, FocusHandle, InteractiveElement, IntoElement, ParentElement, Styled, Window,
    div, px,
};

use crate::app::button;
use crate::theme;

/// What the daemon requires be shown before this continuation, and the
/// decision it belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Disclosure {
    pub session_id: String,
    /// The daemon's words, shown unchanged.
    pub text: String,
    /// The decision those words belong to; carried back so the person's yes
    /// is a yes to this one and not to a later one.
    pub revision: String,
}

/// The overlay, over whatever the window was showing.
pub fn render(
    disclosure: &Disclosure,
    focus: &FocusHandle,
    on_continue: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_cancel: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(gpui::hsla(0., 0., 0., 0.55))
        .child(
            div()
                .id("disclosure")
                .track_focus(focus)
                .key_context("Overlay Disclosure")
                .w(px(520.))
                .p_4()
                .flex()
                .flex_col()
                .gap_3()
                .rounded(px(8.))
                .bg(theme::panel_bg())
                .border_1()
                .border_color(theme::border())
                .text_color(theme::text())
                .child(div().text_size(px(15.)).child("Continue in Corral"))
                .child(
                    div()
                        .text_color(theme::muted())
                        .child(disclosure.text.clone()),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .justify_end()
                        .child(button("disclosure-cancel", "Not now", on_cancel))
                        .child(button(
                            "disclosure-continue",
                            "Continue anyway",
                            on_continue,
                        )),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(theme::muted())
                        .child("y continues · esc does not"),
                ),
        )
}
