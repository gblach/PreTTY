use std::fs;
use std::path::PathBuf;

use alacritty_terminal::term::{Config, Osc52};
use alacritty_terminal::vte::ansi::{CursorShape as TermCursorShape, CursorStyle};
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants};
use gpui_kit::component::group_box::{GroupBox, GroupBoxVariants};
use gpui_kit::component::input::{InputEvent, InputState, NumberInput};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::select::{SearchableVec, Select, SelectEvent, SelectState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme, IndexPath, Selectable, Sizable, StyledExt, h_flex, v_flex};
use gpui_kit::*;
use serde::{Deserialize, Serialize};

use crate::terminal::cell_size;

const MIN_FONT_SIZE: f32 = 6.;
const MAX_FONT_SIZE: f32 = 72.;
const MIN_COLUMNS: f32 = 20.;
const MAX_COLUMNS: f32 = 1000.;
const MIN_ROWS: f32 = 5.;
const MAX_ROWS: f32 = 500.;
const MIN_OPACITY: f32 = 50.;
const MAX_OPACITY: f32 = 100.;
/// Rough room taken by the tab bar and padding. The window is corrected to the
/// exact grid size once its layout is known, so this only needs to be close.
const CHROME_ESTIMATE: Size<Pixels> = size(px(8.), px(48.));

#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CursorShape {
    #[default]
    Underline,
    Beam,
    Block,
}

impl CursorShape {
    const ALL: [Self; 3] = [Self::Underline, Self::Beam, Self::Block];

    fn label(self) -> &'static str {
        match self {
            Self::Underline => "Underline",
            Self::Beam => "Beam",
            Self::Block => "Block",
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FontSettings {
    /// Empty means the theme's monospace font.
    pub family: String,
    pub size: f32,
}

impl Default for FontSettings {
    fn default() -> Self {
        Self {
            family: String::new(),
            size: 14.,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CursorSettings {
    pub shape: CursorShape,
    pub blink: bool,
}

impl Default for CursorSettings {
    fn default() -> Self {
        Self {
            shape: CursorShape::Underline,
            blink: true,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowSettings {
    pub columns: u16,
    pub rows: u16,
    pub maximized: bool,
    pub titlebar: bool,
}

impl Default for WindowSettings {
    fn default() -> Self {
        Self {
            columns: 100,
            rows: 30,
            maximized: true,
            titlebar: false,
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub font: FontSettings,
    pub cursor: CursorSettings,
    pub window: WindowSettings,
    pub background: BackgroundSettings,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BackgroundSettings {
    /// Percent; 100 is fully opaque.
    pub opacity: u8,
    /// Shade every other row.
    pub zebra: bool,
    /// Flash the terminal on a bell and mark background tabs that rang.
    pub visual_bell: bool,
}

impl Default for BackgroundSettings {
    fn default() -> Self {
        Self {
            opacity: 100,
            zebra: false,
            visual_bell: true,
        }
    }
}

impl Global for Settings {}

fn config_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("pretty.toml"))
}

impl Settings {
    pub fn load(cx: &App) -> Self {
        let mut settings: Settings = config_path()
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|text| {
                toml::from_str(&text)
                    .inspect_err(|err| eprintln!("invalid settings file: {err}"))
                    .ok()
            })
            .unwrap_or_default();
        let font = &mut settings.font;
        if font.family.is_empty() {
            font.family = cx.theme().mono_font_family.to_string();
        }
        font.size = font.size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
        let window = &mut settings.window;
        window.columns = window.columns.clamp(MIN_COLUMNS as u16, MAX_COLUMNS as u16);
        window.rows = window.rows.clamp(MIN_ROWS as u16, MAX_ROWS as u16);
        let background = &mut settings.background;
        background.opacity = background
            .opacity
            .clamp(MIN_OPACITY as u8, MAX_OPACITY as u8);
        settings
    }

    fn save(&self) {
        let Some(path) = config_path() else {
            return;
        };
        let result = toml::to_string(self)
            .map_err(|err| err.to_string())
            .and_then(|text| fs::write(&path, text).map_err(|err| err.to_string()));
        if let Err(err) = result {
            eprintln!("failed to save settings to {}: {err}", path.display());
        }
    }

    fn update(cx: &mut App, f: impl FnOnce(&mut Settings)) {
        cx.update_global::<Settings, _>(|settings, _| {
            f(settings);
            settings.save();
        });
    }

    pub fn opacity(&self) -> f32 {
        self.background.opacity as f32 / 100.
    }

    pub fn window_decorations(&self) -> WindowDecorations {
        if self.window.titlebar {
            WindowDecorations::Server
        } else {
            WindowDecorations::Client
        }
    }

    pub fn window_background(&self) -> WindowBackgroundAppearance {
        if self.background.opacity < 100 {
            WindowBackgroundAppearance::Transparent
        } else {
            WindowBackgroundAppearance::Opaque
        }
    }

    pub fn window_chars(&self) -> (u16, u16) {
        (self.window.columns, self.window.rows)
    }

    pub fn window_bounds(&self, cx: &App) -> WindowBounds {
        let cell = cell_size(cx.text_system(), self);
        let window_size = size(
            cell.width * self.window.columns as f32 + CHROME_ESTIMATE.width,
            cell.height * self.window.rows as f32 + CHROME_ESTIMATE.height,
        );
        let bounds = Bounds::centered(None, window_size, cx);
        if self.window.maximized {
            WindowBounds::Maximized(bounds)
        } else {
            WindowBounds::Windowed(bounds)
        }
    }

    pub fn term_config(&self) -> Config {
        let shape = match self.cursor.shape {
            CursorShape::Block => TermCursorShape::Block,
            CursorShape::Beam => TermCursorShape::Beam,
            CursorShape::Underline => TermCursorShape::Underline,
        };
        Config {
            default_cursor_style: CursorStyle {
                shape,
                blinking: self.cursor.blink,
            },
            // Programs may set the clipboard but never read it: reading would
            // hand its contents to anything printed, e.g. by a remote host.
            osc52: Osc52::OnlyCopy,
            ..Default::default()
        }
    }
}

type FontList = SearchableVec<SharedString>;

/// gpui exposes no fixed-pitch flag, so compare the widths of a narrow, a wide
/// and a medium glyph. Fonts missing these glyphs (symbols, emoji) are skipped.
fn is_monospace(text_system: &TextSystem, family: &str) -> bool {
    let font_id = text_system.resolve_font(&font(family.to_string()));
    let widths: Option<Vec<Pixels>> = ['i', 'W', 'm']
        .into_iter()
        .map(|ch| {
            text_system
                .advance(font_id, px(16.), ch)
                .ok()
                .map(|size| size.width)
        })
        .collect();
    widths.is_some_and(|widths| widths.iter().all(|width| *width == widths[0]))
}

/// The input's value, if it is a number within `min..=max`. Typing produces
/// partial values like "1" on the way to "12", which are ignored.
fn number_value(state: &Entity<InputState>, min: f32, max: f32, cx: &App) -> Option<f32> {
    let value = state.read(cx).value().parse::<f32>().ok()?;
    (min..=max).contains(&value).then_some(value)
}

fn number_input(
    value: f32,
    min: f32,
    max: f32,
    step: f64,
    window: &mut Window,
    cx: &mut App,
) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .default_value(value.to_string())
            .min(min as f64)
            .max(max as f64)
            .step(step)
    })
}

pub struct SettingsPanel {
    font_family: Entity<SelectState<FontList>>,
    font_size: Entity<InputState>,
    window_columns: Entity<InputState>,
    window_rows: Entity<InputState>,
    opacity: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl SettingsPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = cx.global::<Settings>().clone();

        let text_system = cx.text_system().clone();
        let mut fonts = text_system.all_font_names();
        fonts.retain(|family| is_monospace(&text_system, family));
        fonts.sort();
        fonts.dedup();
        let selected = fonts
            .iter()
            .position(|name| *name == settings.font.family)
            .map(IndexPath::new);
        let fonts = SearchableVec::new(
            fonts
                .into_iter()
                .map(SharedString::from)
                .collect::<Vec<_>>(),
        );
        let font_family =
            cx.new(|cx| SelectState::new(fonts, selected, window, cx).searchable(true));

        let size = settings.font.size;
        let font_size = number_input(size, MIN_FONT_SIZE, MAX_FONT_SIZE, 1., window, cx);
        let columns = settings.window.columns as f32;
        let window_columns = number_input(columns, MIN_COLUMNS, MAX_COLUMNS, 1., window, cx);
        let rows = settings.window.rows as f32;
        let window_rows = number_input(rows, MIN_ROWS, MAX_ROWS, 1., window, cx);
        let percent = settings.background.opacity as f32;
        let opacity = number_input(percent, MIN_OPACITY, MAX_OPACITY, 5., window, cx);

        let subscriptions = vec![
            cx.subscribe(&font_family, |_, _, event: &SelectEvent<FontList>, cx| {
                if let SelectEvent::Confirm(Some(family)) = event {
                    Settings::update(cx, |settings| settings.font.family = family.to_string());
                }
            }),
            cx.subscribe(&font_size, |_, state, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change)
                    && let Some(size) = number_value(&state, MIN_FONT_SIZE, MAX_FONT_SIZE, cx)
                {
                    Settings::update(cx, |settings| settings.font.size = size);
                }
            }),
            cx.subscribe(&window_columns, |_, state, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change)
                    && let Some(columns) = number_value(&state, MIN_COLUMNS, MAX_COLUMNS, cx)
                {
                    Settings::update(cx, |settings| settings.window.columns = columns as u16);
                }
            }),
            cx.subscribe(&window_rows, |_, state, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change)
                    && let Some(rows) = number_value(&state, MIN_ROWS, MAX_ROWS, cx)
                {
                    Settings::update(cx, |settings| settings.window.rows = rows as u16);
                }
            }),
            cx.subscribe(&opacity, |_, state, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change)
                    && let Some(opacity) = number_value(&state, MIN_OPACITY, MAX_OPACITY, cx)
                {
                    Settings::update(cx, |settings| settings.background.opacity = opacity as u8);
                }
            }),
            cx.observe_global::<Settings>(|_, cx| cx.notify()),
        ];

        Self {
            font_family,
            font_size,
            window_columns,
            window_rows,
            opacity,
            _subscriptions: subscriptions,
        }
    }
}

/// A settings row: name and a short explanation on the left, the control
/// right-aligned, so every value lines up in one column.
fn row(title: &'static str, description: &'static str, control: impl IntoElement, cx: &App) -> Div {
    h_flex()
        .gap_6()
        .py_3()
        .justify_between()
        .child(
            v_flex()
                .gap_1()
                .min_w_0()
                .child(div().text_sm().font_medium().child(title))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(description),
                ),
        )
        .child(h_flex().flex_shrink_0().justify_end().child(control))
}

pub fn section(title: &'static str, cx: &App) -> GroupBox {
    GroupBox::new()
        .outline()
        .title(div().text_xs().font_semibold().child(title))
        .content_style(
            StyleRefinement::default()
                .gap_0()
                .py_1()
                .rounded_none()
                .bg(cx.theme().secondary),
        )
}

impl Render for SettingsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = cx.global::<Settings>();

        let cursor_shape = ButtonGroup::new("cursor-shape")
            .small()
            .children(CursorShape::ALL.map(|shape| {
                let button = Button::new(shape.label())
                    .label(shape.label())
                    .cursor_pointer();
                if shape == settings.cursor.shape {
                    button.primary().selected(true)
                } else {
                    button.outline()
                }
            }))
            .on_click(|clicked, _, cx| {
                if let Some(&index) = clicked.first() {
                    Settings::update(cx, |settings| {
                        settings.cursor.shape = CursorShape::ALL[index]
                    });
                }
            });
        let cursor_blink = Switch::new("cursor-blink")
            .cursor_pointer()
            .checked(settings.cursor.blink)
            .on_click(|checked, _, cx| {
                Settings::update(cx, |settings| settings.cursor.blink = *checked);
            });
        let window_size = h_flex()
            .gap_2()
            .child(
                div()
                    .w(px(120.))
                    .child(NumberInput::new(&self.window_columns)),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("×"),
            )
            .child(div().w(px(120.)).child(NumberInput::new(&self.window_rows)));
        let titlebar = Switch::new("titlebar")
            .cursor_pointer()
            .checked(settings.window.titlebar)
            .on_click(|checked, _, cx| {
                Settings::update(cx, |settings| settings.window.titlebar = *checked);
            });
        let maximized = Switch::new("maximized")
            .cursor_pointer()
            .checked(settings.window.maximized)
            .on_click(|checked, window, cx| {
                Settings::update(cx, |settings| settings.window.maximized = *checked);
                if window.is_maximized() != *checked {
                    window.zoom_window();
                }
            });
        let bell = Switch::new("visual-bell")
            .cursor_pointer()
            .checked(settings.background.visual_bell)
            .on_click(|checked, _, cx| {
                Settings::update(cx, |settings| settings.background.visual_bell = *checked);
            });
        let font_family = div().w(px(220.)).child(Select::new(&self.font_family));
        let font_size = div().w(px(140.)).child(NumberInput::new(&self.font_size));
        let opacity = div().w(px(140.)).child(NumberInput::new(&self.opacity));
        let zebra = Switch::new("zebra")
            .cursor_pointer()
            .checked(settings.background.zebra)
            .on_click(|checked, _, cx| {
                Settings::update(cx, |settings| settings.background.zebra = *checked);
            });

        v_flex()
            .gap_5()
            .pr_2()
            .max_h(window.viewport_size().height * 0.7)
            .child(
                section("FONT", cx)
                    .child(row("Family", "Installed monospace fonts", font_family, cx))
                    .child(row("Size", "Text size in pixels", font_size, cx)),
            )
            .child(
                section("CURSOR", cx)
                    .child(row(
                        "Shape",
                        "Programs like vim can still change it",
                        cursor_shape,
                        cx,
                    ))
                    .child(row(
                        "Blinking",
                        "Blink the cursor while idle",
                        cursor_blink,
                        cx,
                    )),
            )
            .child(
                section("WINDOW", cx)
                    .child(row(
                        "Size",
                        "Columns × rows of the terminal",
                        window_size,
                        cx,
                    ))
                    .child(row("Maximized", "Open the window maximized", maximized, cx))
                    .child(row(
                        "Title bar",
                        "When hidden, drag a tab to move the window",
                        titlebar,
                        cx,
                    )),
            )
            .child(
                section("BACKGROUND", cx)
                    .child(row(
                        "Opacity",
                        "Terminal background, 50-100%; 100 is opaque",
                        opacity,
                        cx,
                    ))
                    .child(row(
                        "Zebra stripes",
                        "Shade every other row for easier reading",
                        zebra,
                        cx,
                    ))
                    .child(row(
                        "Visual bell",
                        "Flash the terminal and mark background tabs",
                        bell,
                        cx,
                    )),
            )
            .overflow_y_scrollbar()
    }
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::term::Osc52;

    use super::{CursorShape, Settings};

    #[test]
    fn programs_cannot_read_the_clipboard() {
        assert_eq!(Settings::default().term_config().osc52, Osc52::OnlyCopy);
    }

    #[test]
    fn partial_file_falls_back_to_defaults() {
        let settings: Settings = toml::from_str("[cursor]\nshape = \"block\"").unwrap();
        assert!(settings.cursor.shape == CursorShape::Block);
        assert_eq!(settings.font.size, 14.);
        assert!(settings.cursor.blink);
        assert!(settings.window.maximized);
    }

    #[test]
    fn round_trips_through_toml() {
        let mut settings = Settings::default();
        settings.font.family = "Fira Code".into();
        settings.font.size = 16.;
        settings.cursor.shape = CursorShape::Block;
        let text = toml::to_string(&settings).unwrap();
        assert!(text.contains("[font]\nfamily = \"Fira Code\""));
        assert!(text.contains("[cursor]\nshape = \"block\""));
        assert!(text.contains("[window]\ncolumns = 100"));
        let loaded: Settings = toml::from_str(&text).unwrap();
        assert_eq!(loaded.font.family, "Fira Code");
        assert_eq!(loaded.font.size, 16.);
        assert!(loaded.cursor.shape == CursorShape::Block);
        assert!(loaded.cursor.blink);
    }
}
