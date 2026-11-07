use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonRounded, ButtonVariants};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{
    ActiveTheme, IconName, Sizable, TitleBar, WindowExt, h_flex, v_flex, window_border,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::about;
use crate::settings::{Settings, SettingsPanel};
use crate::terminal::{Shell, TerminalEvent, TerminalView};

actions!(
    pretty,
    [
        NewTab,
        CloseTab,
        NextTab,
        PrevTab,
        MoveTabLeft,
        MoveTabRight,
        OpenSettings,
        ShowAbout
    ]
);

/// Switches to the tab at the given zero-based index.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = pretty, no_json)]
pub struct ActivateTab(pub usize);

struct TerminalTab {
    view: Entity<TerminalView>,
    /// A bell rang while this tab was in the background.
    bell: bool,
    _subscription: Subscription,
}

/// Drag payload for reordering tabs; it also renders as the preview under the
/// cursor.
#[derive(Clone)]
struct DraggedTab {
    index: usize,
    title: SharedString,
}

impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .py_1()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .text_color(cx.theme().tab_active_foreground)
            .child(self.title.clone())
    }
}

pub struct Workspace {
    tabs: Vec<TerminalTab>,
    active: usize,
    /// The startup size is a pixel estimate; correct it after the first layout.
    fit_pending: bool,
    window_chars: (u16, u16),
    window_background: WindowBackgroundAppearance,
    window_decorations: WindowDecorations,
    /// Where the left button went down on the tab bar; moving far enough from
    /// it while the title bar is hidden drags the window.
    drag_start: Option<Point<Pixels>>,
    /// Set while the window is briefly unmaximized after the title bar was
    /// toggled; maximizing again makes the compositor send the new size.
    remaximize: bool,
    _settings: Subscription,
    _bounds: Subscription,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = cx.observe_global_in::<Settings>(window, |this, window, cx| {
            let background = cx.global::<Settings>().window_background();
            if background != this.window_background {
                this.window_background = background;
                window.set_background_appearance(background);
            }
            let decorations = cx.global::<Settings>().window_decorations();
            if decorations != this.window_decorations {
                this.window_decorations = decorations;
                window.request_decorations(decorations);
                // COSMIC keeps the old size of a maximized window when its
                // title bar is added or removed.
                if window.is_maximized() {
                    this.remaximize = true;
                    window.zoom_window();
                }
            }
            let chars = cx.global::<Settings>().window_chars();
            if chars != this.window_chars {
                this.window_chars = chars;
                this.fit_window(window, cx);
            }
        });
        let bounds = cx.observe_window_bounds(window, |this, window, _| {
            if this.remaximize && !window.is_maximized() {
                this.remaximize = false;
                window.zoom_window();
            }
        });
        let mut this = Self {
            tabs: Vec::new(),
            active: 0,
            fit_pending: true,
            window_chars: cx.global::<Settings>().window_chars(),
            window_background: cx.global::<Settings>().window_background(),
            window_decorations: cx.global::<Settings>().window_decorations(),
            drag_start: None,
            remaximize: false,
            _settings: settings,
            _bounds: bounds,
        };
        this.new_tab(window, cx);
        this
    }

    /// Resizes the window by the difference between the active terminal's grid
    /// and the configured size, so the tab bar and padding need not be known.
    fn fit_window(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        if window.is_maximized() {
            return;
        }
        let (columns, rows) = self.window_chars;
        let (current_columns, current_rows, cell) = tab.view.read(cx).grid();
        let viewport = window.viewport_size();
        window.resize(size(
            viewport.width + cell.width * (columns as f32 - current_columns as f32),
            viewport.height + cell.height * (rows as f32 - current_rows as f32),
        ));
    }

    fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        tab.view.focus_handle(cx).focus(window, cx);
        self.tabs[index].bell = false;
        self.active = index;
        cx.notify();
    }

    fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            cx.quit();
            return;
        }
        if self.active > index || self.active == self.tabs.len() {
            self.active -= 1;
        }
        self.activate(self.active, window, cx);
    }

    fn close_view(
        &mut self,
        view: &Entity<TerminalView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.tabs.iter().position(|tab| &tab.view == view) {
            self.close_tab(index, window, cx);
        }
    }

    fn mark_bell(&mut self, view: &Entity<TerminalView>, cx: &mut Context<Self>) {
        let active = self.active;
        if let Some((_, tab)) = self
            .tabs
            .iter_mut()
            .enumerate()
            .find(|(index, tab)| *index != active && &tab.view == view)
        {
            tab.bell = true;
            cx.notify();
        }
    }

    fn new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let working_directory = self
            .tabs
            .get(self.active)
            .and_then(|tab| tab.view.read(cx).working_directory());
        let view = match Shell::spawn(cx.global::<Settings>().term_config(), working_directory) {
            Ok(shell) => cx.new(|cx| TerminalView::new(shell, cx)),
            Err(err) => {
                eprintln!("failed to start shell: {err}");
                return;
            }
        };
        let subscription =
            cx.subscribe_in(&view, window, |this, view, event, window, cx| match event {
                TerminalEvent::TitleChanged => cx.notify(),
                TerminalEvent::GridResized => {
                    if this.fit_pending {
                        this.fit_pending = false;
                        this.fit_window(window, cx);
                    }
                }
                TerminalEvent::Bell => this.mark_bell(view, cx),
                TerminalEvent::Exited => this.close_view(view, window, cx),
            });
        self.tabs.push(TerminalTab {
            view,
            bell: false,
            _subscription: subscription,
        });
        self.activate(self.tabs.len() - 1, window, cx);
    }

    fn cycle(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        let len = self.tabs.len() as isize;
        let index = (self.active as isize + step).rem_euclid(len) as usize;
        self.activate(index, window, cx);
    }

    fn move_tab(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        if from == to || from >= self.tabs.len() || to >= self.tabs.len() {
            return;
        }
        let active = self.tabs[self.active].view.clone();
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        self.active = self
            .tabs
            .iter()
            .position(|tab| tab.view == active)
            .unwrap_or(0);
        cx.notify();
    }

    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panel = cx.new(|cx| SettingsPanel::new(window, cx));
        // The theme background is transparent so the window can be see-through;
        // the dialog needs an opaque surface of its own.
        window.open_dialog(cx, move |dialog, _, cx| {
            let about = Button::new("about")
                .cursor_pointer()
                .outline()
                .label("About & shortcuts")
                .on_click(|_, window, cx| {
                    window.close_dialog(cx);
                    about::open(window, cx);
                });
            dialog
                .title("Settings")
                .w(px(560.))
                .rounded_none()
                .bg(cx.theme().popover)
                .child(panel.clone())
                .footer(DialogFooter::new().child(about))
        });
    }

    fn render_tab_bar(&self, cx: &mut Context<Self>) -> TabBar {
        // Same hover as the ghost + and settings buttons in dark mode.
        let accent = cx.theme().accent;
        let close_style = ButtonCustomVariant::new(cx)
            .hover(accent.opacity(0.5))
            .active(accent);
        let tabs = self.tabs.iter().enumerate().map(|(index, tab)| {
            let close = Button::new(("close", index))
                .cursor_pointer()
                .custom(close_style)
                .rounded(ButtonRounded::None)
                .small()
                .icon(IconName::Close)
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.close_tab(index, window, cx);
                }));
            let title = tab.view.read(cx).title();
            let dragged = DraggedTab {
                index,
                title: title.clone(),
            };
            Tab::new()
                .cursor_pointer()
                .flex_1()
                .when(tab.bell, |this| {
                    this.prefix(div().pl_2().child(gpui_kit::assets::IconName::Bell))
                })
                .label(title)
                .suffix(close)
                .on_drag(dragged, |dragged, _, _, cx| {
                    cx.stop_propagation();
                    cx.new(|_| dragged.clone())
                })
                // The dropped tab takes this tab's place, so the marker goes on
                // the side it will land: right when moving right, left otherwise.
                .drag_over::<DraggedTab>(move |style, dragged, _, cx| {
                    let style = style.border_color(cx.theme().drag_border);
                    if dragged.index < index {
                        style.border_r_2()
                    } else {
                        style.border_l_2()
                    }
                })
                .on_drop(cx.listener(move |this, dragged: &DraggedTab, _, cx| {
                    this.move_tab(dragged.index, index, cx)
                }))
        });
        let new_tab = Button::new("new-tab")
            .cursor_pointer()
            .ghost()
            .rounded(ButtonRounded::None)
            .icon(IconName::Plus)
            .on_click(cx.listener(|this, _, window, cx| this.new_tab(window, cx)));
        let settings = Button::new("settings")
            .cursor_pointer()
            .ghost()
            .rounded(ButtonRounded::None)
            .icon(gpui_kit::assets::IconName::SlidersHorizontal)
            .on_click(cx.listener(|this, _, window, cx| this.open_settings(window, cx)));

        TabBar::new("tabs")
            .selected_index(self.active)
            // Any max width switches tabs into ellipsis mode; tabs are sized by flex_1, not a cap.
            .max_width(Pixels::MAX)
            .on_click(
                cx.listener(|this, index: &usize, window, cx| this.activate(*index, window, cx)),
            )
            .children(tabs)
            .last_empty_space(div())
            .suffix(h_flex().child(new_tab).child(settings))
    }
}

impl Workspace {
    /// Without a title bar the tab bar moves the window: a press followed by a
    /// small drag starts a move, so a plain click still switches tabs.
    fn render_window_drag(&self, tab_bar: TabBar, cx: &mut Context<Self>) -> Div {
        const THRESHOLD: Pixels = px(4.);
        let bar = v_flex().child(tab_bar);
        if cx.global::<Settings>().window.titlebar {
            return bar;
        }
        bar.capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, window, _| {
            match event.button {
                MouseButton::Left if event.click_count == 2 => {
                    this.drag_start = None;
                    window.zoom_window();
                }
                MouseButton::Left => this.drag_start = Some(event.position),
                MouseButton::Right => window.show_window_menu(event.position),
                _ => {}
            }
        }))
        .capture_any_mouse_up(cx.listener(|this, _: &MouseUpEvent, _, _| {
            this.drag_start = None;
        }))
        .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
            let Some(start) = this.drag_start else {
                return;
            };
            // A tab drag starts at a smaller distance and its handler runs
            // first, so dragging a tab reorders instead of moving the window.
            if event.pressed_button != Some(MouseButton::Left) || cx.has_active_drag() {
                this.drag_start = None;
                return;
            }
            let moved = event.position - start;
            if moved.x.abs() > THRESHOLD || moved.y.abs() > THRESHOLD {
                this.drag_start = None;
                window.start_window_move();
            }
        }))
    }

    fn render_content(
        &mut self,
        title_bar: Option<TitleBar>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .size_full()
            .on_action(cx.listener(|this, _: &NewTab, window, cx| this.new_tab(window, cx)))
            .on_action(
                cx.listener(|this, _: &CloseTab, window, cx| {
                    this.close_tab(this.active, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &NextTab, window, cx| this.cycle(1, window, cx)))
            .on_action(cx.listener(|this, _: &PrevTab, window, cx| this.cycle(-1, window, cx)))
            .on_action(cx.listener(|this, _: &MoveTabLeft, _, cx| {
                this.move_tab(this.active, this.active.saturating_sub(1), cx)
            }))
            .on_action(cx.listener(|this, _: &MoveTabRight, _, cx| {
                this.move_tab(this.active, this.active + 1, cx)
            }))
            .on_action(|_: &ShowAbout, window, cx| about::open(window, cx))
            .on_action(
                cx.listener(|this, _: &OpenSettings, window, cx| this.open_settings(window, cx)),
            )
            .on_action(cx.listener(|this, action: &ActivateTab, window, cx| {
                this.activate(action.0, window, cx)
            }))
            .children(title_bar)
            .child(self.render_window_drag(self.render_tab_bar(cx), cx))
            .children(self.tabs.get(self.active).map(|tab| tab.view.clone()))
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut window_title = SharedString::from("PreTTY");
        if let Some(tab) = self.tabs.get(self.active) {
            let title = tab.view.read(cx).title();
            window_title = if self.tabs.len() > 1 {
                format!("{title} - PreTTY [{}/{}]", self.active + 1, self.tabs.len())
            } else {
                format!("{title} - PreTTY")
            }
            .into();
            window.set_window_title(&window_title);
        }
        // Compositors without server-side decorations (GNOME on Wayland) give
        // the window no title bar, so draw one when it was asked for.
        let client_decorated = matches!(window.window_decorations(), Decorations::Client { .. });
        let title_bar = (cx.global::<Settings>().window.titlebar && client_decorated).then(|| {
            TitleBar::new()
                .bg(cx.theme().tab_bar)
                .child(div().text_sm().child(window_title))
        });
        // Draws resizable edges and a shadow only while the window uses
        // client-side decorations; otherwise a no-op.
        window_border().child(self.render_content(title_bar, cx))
    }
}
