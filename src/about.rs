use std::sync::Arc;

use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::{ActiveTheme, StyledExt, WindowExt, h_flex, v_flex};
use gpui_kit::*;

use crate::settings::section;
use crate::terminal::{
    Copy, Paste, PastePrimary, ScrollLineDown, ScrollLineUp, ScrollPageDown, ScrollPageUp,
    ScrollToBottom, ScrollToTop,
};
use crate::workspace::{
    ActivateTab, CloseTab, MoveTabLeft, MoveTabRight, NewTab, NextTab, OpenSettings, PrevTab,
    ShowAbout,
};

/// Key context of the terminal view; global bindings match it too, so looking
/// keys up here finds both kinds.
const CONTEXT: &str = "Terminal";
/// Pre-rendered at 2x the displayed 72px: the GPU downscales images without
/// smoothing, so the 512px icon would look jagged here.
const ICON: &[u8] = include_bytes!("../assets/pretty-about.png");

/// Looks the keys up in the live keymap so this list cannot drift from the
/// bindings in `main.rs`.
fn keys(action: &dyn Action, window: &Window) -> AnyElement {
    match Kbd::binding_for_action(action, Some(CONTEXT), window) {
        Some(kbd) => kbd.into_any_element(),
        None => div().child("-").into_any_element(),
    }
}

fn shortcut(label: &'static str, keys: impl IntoElement) -> Div {
    h_flex()
        .py_2()
        .justify_between()
        .child(div().text_sm().child(label))
        .child(keys)
}

fn mouse_action(label: &'static str, action: &'static str, cx: &App) -> Div {
    shortcut(
        label,
        div()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(action),
    )
}

fn render_about(window: &mut Window, cx: &mut App) -> Div {
    let tab_keys = h_flex()
        .gap_1()
        .child(keys(&ActivateTab(0), window))
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("…"),
        )
        .child(keys(&ActivateTab(9), window));
    let shortcuts = [
        ("New tab", keys(&NewTab, window)),
        ("Close tab", keys(&CloseTab, window)),
        ("Next tab", keys(&NextTab, window)),
        ("Previous tab", keys(&PrevTab, window)),
        ("Move tab left", keys(&MoveTabLeft, window)),
        ("Move tab right", keys(&MoveTabRight, window)),
        ("Go to tab 1-10", tab_keys.into_any_element()),
        ("Copy selection", keys(&Copy, window)),
        ("Paste", keys(&Paste, window)),
        ("Paste selection", keys(&PastePrimary, window)),
        ("Scroll page up", keys(&ScrollPageUp, window)),
        ("Scroll page down", keys(&ScrollPageDown, window)),
        ("Scroll line up", keys(&ScrollLineUp, window)),
        ("Scroll line down", keys(&ScrollLineDown, window)),
        ("Scroll to top", keys(&ScrollToTop, window)),
        ("Scroll to bottom", keys(&ScrollToBottom, window)),
        ("Settings", keys(&OpenSettings, window)),
        ("About", keys(&ShowAbout, window)),
    ];
    let mouse_actions = [
        ("Select text", "Drag"),
        ("Select word", "Double click"),
        ("Select line", "Triple click"),
        ("Copy selection", "Right click"),
        ("Paste selection", "Middle click"),
        ("Reorder tabs", "Drag tab"),
    ];

    v_flex()
        .gap_5()
        .child(
            v_flex()
                .gap_1()
                .items_center()
                .child(
                    img(Arc::new(Image::from_bytes(ImageFormat::Png, ICON.to_vec()))).size(px(72.)),
                )
                .child(div().text_xl().font_semibold().child("PreTTY"))
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(concat!("Version ", env!("CARGO_PKG_VERSION"))),
                ),
        )
        .child(
            section("KEYBOARD SHORTCUTS", cx).children(
                shortcuts
                    .into_iter()
                    .map(|(label, keys)| shortcut(label, keys)),
            ),
        )
        .child(
            section("MOUSE", cx).children(
                mouse_actions
                    .into_iter()
                    .map(|(label, action)| mouse_action(label, action, cx)),
            ),
        )
}

pub fn open(window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, |dialog, window, cx| {
        dialog
            .title("About")
            .w(px(440.))
            .rounded_none()
            .bg(cx.theme().popover)
            .child(render_about(window, cx))
    });
}
