mod about;
mod session;
mod settings;
mod terminal;
mod workspace;

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;

use crate::settings::Settings;
use crate::terminal::{
    BACKGROUND, Copy, Paste, PastePrimary, ScrollLineDown, ScrollLineUp, ScrollPageDown,
    ScrollPageUp, ScrollToBottom, ScrollToTop,
};
use crate::workspace::{
    ActivateTab, CloseTab, MoveTabLeft, MoveTabRight, NewTab, NextTab, OpenSettings, PrevTab,
    ShowAbout, Workspace,
};

// Only listed icons are embedded, including the ones gpui-kit's controls use
// internally (e.g. Minus in number inputs, Check in radios); a missing one
// renders as nothing.
gpui_kit::assets::icon_assets!(
    AppAssets,
    [
        Bell,
        Check,
        ChevronDown,
        ChevronLeft,
        ChevronRight,
        Close,
        Inbox,
        Minus,
        Plus,
        Search,
        SlidersHorizontal,
        WindowClose,
        WindowMaximize,
        WindowMinimize,
        WindowRestore,
    ]
);

fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-shift-t", NewTab, None),
        KeyBinding::new("ctrl-shift-w", CloseTab, None),
        KeyBinding::new("ctrl-tab", NextTab, None),
        KeyBinding::new("ctrl-shift-tab", PrevTab, None),
        KeyBinding::new("ctrl-shift-left", MoveTabLeft, None),
        KeyBinding::new("ctrl-shift-right", MoveTabRight, None),
        KeyBinding::new("ctrl-shift-h", ShowAbout, None),
        KeyBinding::new("ctrl-,", OpenSettings, None),
    ]);
    let tab_keys = [
        "alt-1", "alt-2", "alt-3", "alt-4", "alt-5", "alt-6", "alt-7", "alt-8", "alt-9", "alt-0",
    ];
    cx.bind_keys(
        tab_keys
            .into_iter()
            .enumerate()
            .map(|(i, key)| KeyBinding::new(key, ActivateTab(i), None)),
    );
    cx.bind_keys([
        KeyBinding::new("ctrl-shift-c", Copy, Some("Terminal")),
        KeyBinding::new("ctrl-shift-v", Paste, Some("Terminal")),
        KeyBinding::new("shift-insert", PastePrimary, Some("Terminal")),
        KeyBinding::new("shift-pageup", ScrollPageUp, Some("Terminal")),
        KeyBinding::new("shift-pagedown", ScrollPageDown, Some("Terminal")),
        KeyBinding::new("ctrl-shift-up", ScrollLineUp, Some("Terminal")),
        KeyBinding::new("ctrl-shift-down", ScrollLineDown, Some("Terminal")),
        KeyBinding::new("ctrl-shift-home", ScrollToTop, Some("Terminal")),
        KeyBinding::new("ctrl-shift-end", ScrollToBottom, Some("Terminal")),
        // gpui-kit's Root binds these for focus navigation and copy, but a
        // terminal must receive them as raw key input.
        KeyBinding::new("tab", NoAction, Some("Terminal")),
        KeyBinding::new("shift-tab", NoAction, Some("Terminal")),
        KeyBinding::new("ctrl-c", NoAction, Some("Terminal")),
    ]);
}

fn main() {
    gpui_kit::application().with_assets(AppAssets).run(|cx| {
        gpui_kit::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        // gpui-kit's active tab overlaps the bar's bottom border to join the
        // content below, so it must have the terminal's color to line up.
        Theme::update(cx, |theme| {
            theme.colors.tab_active = rgb(BACKGROUND).into();
            theme.colors.tab_active_foreground = rgb(0xffffff).into();
            theme.colors.tab_foreground = rgb(0x808080).into();
            // The window root paints this color under everything; the terminal
            // paints its own background, with the configured opacity.
            theme.colors.background = transparent_black();
        });
        cx.set_global(Settings::load(cx));
        bind_keys(cx);

        let options = WindowOptions {
            window_bounds: Some(cx.global::<Settings>().window_bounds(cx)),
            window_background: cx.global::<Settings>().window_background(),
            window_decorations: Some(cx.global::<Settings>().window_decorations()),
            // Must match assets/pretty.desktop so docks show its name and icon.
            app_id: Some("pretty".into()),
            titlebar: Some(TitlebarOptions {
                title: Some("PreTTY".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| Workspace::new(window, cx))
        })
        .expect("failed to open window");
        cx.activate(true);
    });
}
