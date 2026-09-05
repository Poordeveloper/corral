//! A single line of text a person types: enough for a directory and an
//! argument list, and nothing gpui does not already provide.
//!
//! Typing appends, Backspace removes, paste inserts the clipboard's text on
//! one line. No caret movement, no selection: the New Session form is three
//! fields, and a text editor is not PR9's (PR9 plan, D2: no gpui-component).

use gpui::prelude::FluentBuilder;
use gpui::{
    ClickEvent, Context, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};

use crate::app::Paste;
use crate::theme;

pub struct TextField {
    text: String,
    placeholder: SharedString,
    focus: FocusHandle,
}

impl TextField {
    pub fn new(text: String, placeholder: &'static str, cx: &mut Context<Self>) -> Self {
        Self {
            text,
            placeholder: placeholder.into(),
            focus: cx.focus_handle(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn set_text(&mut self, text: String, cx: &mut Context<Self>) {
        self.text = text;
        cx.notify();
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    fn key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.modifiers.platform || keystroke.modifiers.control {
            return;
        }
        match keystroke.key.as_str() {
            "backspace" => {
                self.text.pop();
            }
            // Keys the form itself answers, or that mean nothing on one line.
            "enter" | "escape" | "tab" | "up" | "down" | "left" | "right" => return,
            _ => {
                let Some(typed) = keystroke.key_char.as_deref() else {
                    return;
                };
                if typed.chars().any(char::is_control) {
                    return;
                }
                self.text.push_str(typed);
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn paste(&mut self, _: &Paste, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        // One line: a newline in the clipboard is a space here, never a
        // submission.
        let one_line: String = text
            .chars()
            .map(|character| {
                if character.is_control() {
                    ' '
                } else {
                    character
                }
            })
            .collect();
        self.text.push_str(one_line.trim());
        cx.notify();
    }

    fn take_focus(&mut self, _: &ClickEvent, window: &mut Window, _cx: &mut Context<Self>) {
        window.focus(&self.focus);
    }
}

impl Render for TextField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.is_focused(window);
        let shown: SharedString = if self.text.is_empty() {
            self.placeholder.clone()
        } else {
            self.text.clone().into()
        };
        div()
            .id("text-field")
            .track_focus(&self.focus)
            .key_context("TextField")
            .on_action(cx.listener(Self::paste))
            .on_key_down(cx.listener(Self::key_down))
            .on_click(cx.listener(Self::take_focus))
            .h(px(28.))
            .px_2()
            .flex()
            .items_center()
            .rounded(px(4.))
            .border_1()
            .border_color(if focused {
                theme::accent()
            } else {
                theme::border()
            })
            .bg(theme::window_bg())
            .text_color(if self.text.is_empty() {
                theme::muted()
            } else {
                theme::text()
            })
            .child(shown)
            .when(focused, |this| {
                this.child(div().text_color(theme::accent()).child("▏"))
            })
    }
}
