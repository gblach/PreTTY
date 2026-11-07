use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use alacritty_terminal::event::{Event as TermEvent, EventListener, Notify, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, Msg, Notifier};
use alacritty_terminal::grid::{Dimensions, Indexed, Scroll};
use alacritty_terminal::index::{Column, Direction, Line, Point as GridPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term, TermMode, viewport_to_point};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Color, CursorShape, Handler, NamedColor, Rgb};
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui_kit::*;

use crate::session::{self, SessionKind};
use crate::settings::Settings;

actions!(
    terminal,
    [
        Copy,
        Paste,
        PastePrimary,
        ScrollLineDown,
        ScrollLineUp,
        ScrollPageDown,
        ScrollPageUp,
        ScrollToBottom,
        ScrollToTop
    ]
);

const DEFAULT_TITLE: &str = "Terminal";
const LINE_HEIGHT: f32 = 1.3;
const BLINK_INTERVAL: Duration = Duration::from_millis(530);

const ANSI: [u32; 16] = [
    0x1d1f21, 0xcc6666, 0xb5bd68, 0xf0c674, 0x81a2be, 0xb294bb, 0x8abeb7, 0xc5c8c6, 0x666666,
    0xd54e53, 0xb9ca4a, 0xe7c547, 0x7aa6da, 0xc397d8, 0x70c0b1, 0xeaeaea,
];
const FOREGROUND: u32 = 0xc5c8c6;
pub const BACKGROUND: u32 = 0x1d1f21;
const CURSOR: u32 = 0xc5c8c6;
const SELECTION: u32 = 0x373b41;
const ROOT_BACKGROUND: u32 = 0x3d0c0c;
const SSH_BACKGROUND: u32 = 0x0c1a3d;
const CONTAINER_BACKGROUND: u32 = 0x093015;
const SESSION_CHECK_DELAY: Duration = Duration::from_millis(500);
const BELL_FLASH: Duration = Duration::from_millis(150);
const DRAG_SCROLL_INTERVAL: Duration = Duration::from_millis(50);
// The last character may not be punctuation, so a link at the end of a sentence like
// "see https://example.com." does not take the full stop with it.
const LINK_PATTERN: &str = r#"https?://[^\s<>"'`{}|\\^]*[^\s<>"'`{}|\\^.,;:!?)\]]"#;

#[derive(Clone)]
struct Listener(UnboundedSender<TermEvent>);

impl EventListener for Listener {
    fn send_event(&self, event: TermEvent) {
        let _ = self.0.unbounded_send(event);
    }
}

pub enum TerminalEvent {
    TitleChanged,
    GridResized,
    Bell,
    Exited,
}

/// A running shell: the PTY event loop thread plus the terminal state it feeds.
pub struct Shell {
    term: Arc<FairMutex<Term<Listener>>>,
    notifier: Notifier,
    events: UnboundedReceiver<TermEvent>,
    pty: File,
    shell_pid: u32,
}

impl Shell {
    pub fn spawn(config: Config, working_directory: Option<PathBuf>) -> io::Result<Self> {
        let (tx, events) = unbounded();
        let listener = Listener(tx);
        let options = tty::Options {
            working_directory,
            env: HashMap::from([
                ("TERM".into(), "xterm-256color".into()),
                ("COLORTERM".into(), "truecolor".into()),
            ]),
            ..Default::default()
        };
        let window_size = WindowSize {
            num_lines: 24,
            num_cols: 80,
            cell_width: 8,
            cell_height: 16,
        };
        let pty = tty::new(&options, window_size, 0)?;
        let pty_file = pty.file().try_clone()?;
        let shell_pid = pty.child().id();
        let term = Term::new(config, &TermSize::new(80, 24), listener.clone());
        let term = Arc::new(FairMutex::new(term));
        let event_loop = EventLoop::new(term.clone(), listener, pty, false, false)?;
        let notifier = Notifier(event_loop.channel());
        event_loop.spawn();
        Ok(Self {
            term,
            notifier,
            events,
            pty: pty_file,
            shell_pid,
        })
    }
}

fn rgb_from_u32(value: u32) -> Rgb {
    Rgb {
        r: (value >> 16) as u8,
        g: (value >> 8) as u8,
        b: value as u8,
    }
}

fn dim(color: Rgb) -> Rgb {
    let scale = |c: u8| (c as f32 * 0.66) as u8;
    Rgb {
        r: scale(color.r),
        g: scale(color.g),
        b: scale(color.b),
    }
}

/// Palette entry for an index of alacritty's color table: 16 ANSI colors, the
/// 6x6x6 cube, the grayscale ramp, then the named special colors from 256 on.
fn default_rgb(index: usize) -> Rgb {
    let cube_level = |level: usize| {
        if level == 0 {
            0
        } else {
            (55 + level * 40) as u8
        }
    };
    match index {
        0..16 => rgb_from_u32(ANSI[index]),
        16..232 => {
            let i = index - 16;
            Rgb {
                r: cube_level(i / 36),
                g: cube_level(i / 6 % 6),
                b: cube_level(i % 6),
            }
        }
        232..256 => {
            let level = (8 + (index - 232) * 10) as u8;
            Rgb {
                r: level,
                g: level,
                b: level,
            }
        }
        i if i == NamedColor::Background as usize => rgb_from_u32(BACKGROUND),
        i if i == NamedColor::Cursor as usize => rgb_from_u32(CURSOR),
        i if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize).contains(&i) => {
            dim(rgb_from_u32(ANSI[i - NamedColor::DimBlack as usize]))
        }
        i if i == NamedColor::DimForeground as usize => dim(rgb_from_u32(FOREGROUND)),
        _ => rgb_from_u32(FOREGROUND),
    }
}

fn resolve(color: Color, colors: &Colors) -> Rgb {
    let index = match color {
        Color::Spec(rgb) => return rgb,
        Color::Named(name) => name as usize,
        Color::Indexed(index) => index as usize,
    };
    colors[index].unwrap_or_else(|| default_rgb(index))
}

fn to_hsla(color: Rgb) -> Hsla {
    Rgba {
        r: color.r as f32 / 255.,
        g: color.g as f32 / 255.,
        b: color.b as f32 / 255.,
        a: 1.,
    }
    .into()
}

fn control_byte(key: &str) -> Option<u8> {
    let &[byte] = key.as_bytes() else {
        return None;
    };
    match byte {
        b'a'..=b'z' => Some(byte - b'a' + 1),
        b'@' | b'2' => Some(0),
        b'[' | b'3' => Some(0x1b),
        b'\\' | b'4' => Some(0x1c),
        b']' | b'5' => Some(0x1d),
        b'^' | b'6' => Some(0x1e),
        b'_' | b'-' | b'/' | b'7' => Some(0x1f),
        b'8' | b'?' => Some(0x7f),
        _ => None,
    }
}

/// Translates a key press into the byte sequence an xterm-compatible terminal sends.
fn key_to_bytes(keystroke: &Keystroke, mode: TermMode) -> Option<Vec<u8>> {
    let mods = &keystroke.modifiers;
    if mods.platform {
        return None;
    }
    let code = 1 + mods.shift as u8 + 2 * mods.alt as u8 + 4 * mods.control as u8;
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    let cursor_key = |c: char| match (code > 1, app_cursor) {
        (true, _) => format!("\x1b[1;{code}{c}"),
        (false, true) => format!("\x1bO{c}"),
        (false, false) => format!("\x1b[{c}"),
    };
    let tilde = |n: u8| match code > 1 {
        true => format!("\x1b[{n};{code}~"),
        false => format!("\x1b[{n}~"),
    };
    let function = |c: char| match code > 1 {
        true => format!("\x1b[1;{code}{c}"),
        false => format!("\x1bO{c}"),
    };
    let esc_prefix = if mods.alt { "\x1b" } else { "" };

    let sequence = match keystroke.key.as_str() {
        "up" => cursor_key('A'),
        "down" => cursor_key('B'),
        "right" => cursor_key('C'),
        "left" => cursor_key('D'),
        "home" => cursor_key('H'),
        "end" => cursor_key('F'),
        "insert" => tilde(2),
        "delete" => tilde(3),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "f1" => function('P'),
        "f2" => function('Q'),
        "f3" => function('R'),
        "f4" => function('S'),
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        "enter" => format!("{esc_prefix}\r"),
        "tab" if mods.shift => "\x1b[Z".into(),
        "tab" => format!("{esc_prefix}\t"),
        "escape" => "\x1b".into(),
        "backspace" if mods.control => format!("{esc_prefix}\x08"),
        "backspace" => format!("{esc_prefix}\x7f"),
        "space" if mods.control => format!("{esc_prefix}\0"),
        key => {
            if mods.control
                && let Some(byte) = control_byte(key)
            {
                let mut bytes = esc_prefix.as_bytes().to_vec();
                bytes.push(byte);
                return Some(bytes);
            }
            let text = keystroke.key_char.as_deref()?;
            format!("{esc_prefix}{text}")
        }
    };
    Some(sequence.into_bytes())
}

struct BackgroundRect {
    line: usize,
    col: usize,
    cols: usize,
    color: Hsla,
}

/// A run of cells on one line, shaped and painted as a single text line.
struct TextChunk {
    line: usize,
    col: usize,
    text: String,
    runs: Vec<TextRun>,
}

struct Frame {
    rects: Vec<BackgroundRect>,
    chunks: Vec<TextChunk>,
    /// Stripe color and which screen rows get it: those where
    /// `(row + parity) % 2 == 1`.
    stripes: Option<(Hsla, usize)>,
    cursor: Option<(usize, usize, CursorShape, Hsla)>,
}

fn push_rect(rects: &mut Vec<BackgroundRect>, line: usize, col: usize, cols: usize, color: Hsla) {
    if let Some(last) = rects.last_mut()
        && last.line == line
        && last.col + last.cols == col
        && last.color == color
    {
        last.cols += cols;
        return;
    }
    rects.push(BackgroundRect {
        line,
        col,
        cols,
        color,
    });
}

fn push_char(chunk: &mut TextChunk, c: char, run: TextRun) {
    chunk.text.push(c);
    if let Some(last) = chunk.runs.last_mut()
        && last.font == run.font
        && last.color == run.color
        && last.underline == run.underline
        && last.strikethrough == run.strikethrough
    {
        last.len += run.len;
        return;
    }
    chunk.runs.push(run);
}

fn cell_font(base: &Font, flags: Flags) -> Font {
    let mut font = base.clone();
    if flags.contains(Flags::BOLD) {
        font.weight = FontWeight::BOLD;
    }
    if flags.contains(Flags::ITALIC) {
        font.style = FontStyle::Italic;
    }
    font
}

fn stripe_color(color: Rgb) -> Rgb {
    let lift = |c: u8| c.saturating_add(5);
    Rgb {
        r: lift(color.r),
        g: lift(color.g),
        b: lift(color.b),
    }
}

fn visible_links<T>(term: &Term<T>, regex: &mut RegexSearch) -> Vec<Match> {
    let offset = term.grid().display_offset() as i32;
    let start = GridPoint::new(Line(-offset), Column(0));
    let end = GridPoint::new(
        Line(term.screen_lines() as i32 - 1 - offset),
        term.last_column(),
    );
    RegexIter::new(start, end, Direction::Right, term, regex).collect()
}

#[allow(clippy::too_many_arguments)]
fn build_frame(
    term: &Term<Listener>,
    links: &[Match],
    font: &Font,
    focused: bool,
    cursor_on: bool,
    background_override: Option<Rgb>,
    opacity: f32,
    zebra: bool,
) -> Frame {
    let content = term.renderable_content();
    let colors = content.colors;
    let offset = content.display_offset;
    let resolve_cell = |color: Color| match (color, background_override) {
        (Color::Named(NamedColor::Background), Some(background)) => background,
        _ => resolve(color, colors),
    };
    let background = resolve_cell(Color::Named(NamedColor::Background));
    let cursor_color = to_hsla(resolve(Color::Named(NamedColor::Cursor), colors));
    let cursor = content.cursor;
    let block_cursor = focused && cursor_on && cursor.shape == CursorShape::Block;

    let mut rects = Vec::new();
    let mut chunks: Vec<TextChunk> = Vec::new();
    let mut chunk_end = None;

    for indexed in content.display_iter {
        let Indexed { point, cell } = indexed;
        let line = (point.line.0 + offset as i32) as usize;
        let col = point.column.0;
        let flags = cell.flags;
        if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            continue;
        }

        let mut fg = resolve_cell(cell.fg);
        let mut bg = resolve_cell(cell.bg);
        if flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if flags.contains(Flags::DIM) {
            fg = dim(fg);
        }
        let (mut fg, mut bg) = (to_hsla(fg), to_hsla(bg));
        let selected = content
            .selection
            .is_some_and(|s| s.contains_cell(&indexed, cursor.point, cursor.shape));
        if selected {
            bg = to_hsla(rgb_from_u32(SELECTION));
        }
        if block_cursor && point == cursor.point {
            bg = cursor_color;
            fg = to_hsla(background);
        }

        let cols = if flags.contains(Flags::WIDE_CHAR) {
            2
        } else {
            1
        };
        if bg != to_hsla(background) {
            push_rect(&mut rects, line, col, cols, bg);
        }

        // Each chunk is shaped with a fixed per-glyph advance, so a chunk must
        // restart after a wide character to keep later cells on the grid.
        if chunk_end != Some((line, col)) {
            chunks.push(TextChunk {
                line,
                col,
                text: String::new(),
                runs: Vec::new(),
            });
        }
        chunk_end = Some((line, col + cols));
        if cols == 2 {
            chunk_end = None;
        }

        let c = if flags.contains(Flags::HIDDEN) {
            ' '
        } else {
            cell.c
        };
        let decoration_color = Some(fg);
        let link = links.iter().any(|link| link.contains(&point));
        let run = TextRun {
            len: c.len_utf8(),
            font: cell_font(font, flags),
            color: fg,
            background_color: None,
            underline: (link || flags.intersects(Flags::ALL_UNDERLINES)).then_some(
                UnderlineStyle {
                    thickness: px(1.),
                    color: decoration_color,
                    wavy: flags.contains(Flags::UNDERCURL),
                },
            ),
            strikethrough: flags
                .contains(Flags::STRIKEOUT)
                .then_some(StrikethroughStyle {
                    thickness: px(1.),
                    color: decoration_color,
                }),
        };
        let chunk = chunks.last_mut().expect("a chunk was pushed above");
        push_char(chunk, c, run);
    }

    let cursor_line = cursor.point.line.0 + offset as i32;
    let draw_cursor =
        cursor_on && cursor.shape != CursorShape::Hidden && !block_cursor && cursor_line >= 0;
    let cursor = draw_cursor.then_some((
        cursor_line as usize,
        cursor.point.column.0,
        cursor.shape,
        cursor_color,
    ));

    Frame {
        rects,
        chunks,
        // Parity follows the scrollback offset so stripes stay with their text
        // lines while scrolling instead of flickering between rows.
        stripes: zebra.then(|| (to_hsla(stripe_color(background)).alpha(opacity), offset % 2)),
        cursor,
    }
}

fn paint_frame(
    frame: Frame,
    bounds: Bounds<Pixels>,
    cell: Size<Pixels>,
    font_size: Pixels,
    focused: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let cell_origin = |line: usize, col: usize| {
        point(
            bounds.origin.x + cell.width * col as f32,
            bounds.origin.y + cell.height * line as f32,
        )
    };

    if let Some((color, parity)) = frame.stripes {
        let rows = (bounds.size.height / cell.height).ceil() as usize;
        for row in (0..rows).filter(|row| (row + parity) % 2 == 1) {
            let stripe_size = size(bounds.size.width, cell.height);
            window.paint_quad(fill(Bounds::new(cell_origin(row, 0), stripe_size), color));
        }
    }
    for rect in frame.rects {
        let rect_size = size(cell.width * rect.cols as f32, cell.height);
        window.paint_quad(fill(
            Bounds::new(cell_origin(rect.line, rect.col), rect_size),
            rect.color,
        ));
    }

    for chunk in frame.chunks {
        let line = window.text_system().shape_line(
            chunk.text.into(),
            font_size,
            &chunk.runs,
            Some(cell.width),
        );
        let origin = cell_origin(chunk.line, chunk.col);
        let _ = line.paint(origin, cell.height, TextAlign::Left, None, window, cx);
    }

    if let Some((line, col, shape, color)) = frame.cursor {
        let origin = cell_origin(line, col);
        let quad = match shape {
            CursorShape::Beam => fill(Bounds::new(origin, size(px(2.), cell.height)), color),
            CursorShape::Underline => {
                let origin = point(origin.x, origin.y + cell.height - px(2.));
                fill(Bounds::new(origin, size(cell.width, px(2.))), color)
            }
            _ if focused && shape == CursorShape::Block => return,
            _ => outline(Bounds::new(origin, cell), color, BorderStyle::Solid),
        };
        window.paint_quad(quad);
    }
}

pub fn cell_size(text_system: &TextSystem, settings: &Settings) -> Size<Pixels> {
    let font_size = px(settings.font.size);
    let font_id = text_system.resolve_font(&font(settings.font.family.clone()));
    let cell_width = text_system
        .advance(font_id, font_size, 'm')
        .map(|advance| advance.width)
        .unwrap_or(font_size * 0.6);
    size(cell_width, (font_size * LINE_HEIGHT).round())
}

/// Shrinks `bounds` to whole cells and centers them, so the leftover space is
/// split evenly around the grid instead of ending up below and right of it.
fn center_grid(bounds: Bounds<Pixels>, cell: Size<Pixels>) -> Bounds<Pixels> {
    let columns = (bounds.size.width / cell.width).floor().max(2.);
    let rows = (bounds.size.height / cell.height).floor().max(1.);
    let extra = size(
        (bounds.size.width - cell.width * columns).max(px(0.)),
        (bounds.size.height - cell.height * rows).max(px(0.)),
    );
    Bounds::new(
        point(
            bounds.origin.x + extra.width / 2.,
            bounds.origin.y + extra.height / 2.,
        ),
        size(
            bounds.size.width - extra.width,
            bounds.size.height - extra.height,
        ),
    )
}

pub struct TerminalView {
    term: Arc<FairMutex<Term<Listener>>>,
    notifier: Notifier,
    focus_handle: FocusHandle,
    title: SharedString,
    bounds: Bounds<Pixels>,
    cell: Size<Pixels>,
    columns: usize,
    lines: usize,
    scroll_remainder: f32,
    /// Runs while the left button drags a selection, scrolling when the pointer is past an edge.
    drag_scroll: Option<Task<()>>,
    drag_position: Point<Pixels>,
    pty: File,
    shell_pid: u32,
    session: SessionKind,
    session_check_pending: bool,
    /// Blink phase; the cursor is drawn only while this is true.
    cursor_on: bool,
    _events: Task<()>,
    _blink: Task<()>,
    /// Ends the current bell flash; replacing it restarts the flash.
    bell_flash: Option<Task<()>>,
    _settings: Subscription,
    link_regex: RegexSearch,
    /// Links found by the last render, so hovering does not search the screen again.
    links: Vec<Match>,
    hovering_link: bool,
}

impl EventEmitter<TerminalEvent> for TerminalView {}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Drop for TerminalView {
    fn drop(&mut self) {
        let _ = self.notifier.0.send(Msg::Shutdown);
    }
}

impl TerminalView {
    pub fn new(shell: Shell, cx: &mut Context<Self>) -> Self {
        let Shell {
            term,
            notifier,
            mut events,
            pty,
            shell_pid,
        } = shell;
        let events = cx.spawn(async move |this, cx| {
            while let Some(event) = events.next().await {
                if this
                    .update(cx, |this, cx| this.handle_event(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        let blink = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(BLINK_INTERVAL).await;
                if this.update(cx, |this, cx| this.blink(cx)).is_err() {
                    break;
                }
            }
        });
        let settings = cx.observe_global::<Settings>(|this, cx| {
            this.term
                .lock()
                .set_options(cx.global::<Settings>().term_config());
            cx.notify();
        });
        Self {
            term,
            notifier,
            focus_handle: cx.focus_handle(),
            title: DEFAULT_TITLE.into(),
            bounds: Bounds::default(),
            cell: size(px(8.), px(16.)),
            columns: 0,
            lines: 0,
            scroll_remainder: 0.,
            drag_scroll: None,
            drag_position: Point::default(),
            pty,
            shell_pid,
            session: SessionKind::Local,
            session_check_pending: false,
            cursor_on: true,
            _events: events,
            _blink: blink,
            bell_flash: None,
            _settings: settings,
            link_regex: RegexSearch::new(LINK_PATTERN).expect("link pattern is valid"),
            links: Vec::new(),
            hovering_link: false,
        }
    }

    pub fn title(&self) -> SharedString {
        self.title.clone()
    }

    pub fn working_directory(&self) -> Option<PathBuf> {
        session::working_directory(self.shell_pid)
    }

    /// Columns, lines and the pixel size of one cell, as of the last layout.
    pub fn grid(&self) -> (usize, usize, Size<Pixels>) {
        (self.columns, self.lines, self.cell)
    }

    fn window_size(&self) -> WindowSize {
        WindowSize {
            num_lines: self.lines as u16,
            num_cols: self.columns as u16,
            cell_width: f32::from(self.cell.width) as u16,
            cell_height: f32::from(self.cell.height) as u16,
        }
    }

    fn blink(&mut self, cx: &mut Context<Self>) {
        let blinking = self.term.lock().cursor_style().blinking;
        let cursor_on = !blinking || !self.cursor_on;
        if cursor_on != self.cursor_on {
            self.cursor_on = cursor_on;
            cx.notify();
        }
    }

    fn update_session(&mut self, cx: &mut Context<Self>) {
        self.session_check_pending = false;
        // A program may exit without restoring the cursor it set, e.g. a steady one in an editor.
        if session::shell_in_foreground(&self.pty, self.shell_pid) {
            self.term.lock().set_cursor_style(None);
        }
        let session = session::detect(&self.pty, self.shell_pid);
        // A tint is only replaced by one of higher priority and otherwise stays until the shell
        // is back in the foreground, so it does not flicker while child processes come and go.
        if session > self.session || (session == SessionKind::Local && session != self.session) {
            self.session = session;
            cx.notify();
        }
    }

    /// Runs after a delay so a command started by this input (e.g. `ssh` after
    /// Enter) is already running, and at most once per delay while typing or
    /// while output arrives.
    fn schedule_session_check(&mut self, cx: &mut Context<Self>) {
        if self.session_check_pending {
            return;
        }
        self.session_check_pending = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SESSION_CHECK_DELAY).await;
            let _ = this.update(cx, |this, cx| this.update_session(cx));
        })
        .detach();
    }

    fn write(&self, bytes: Vec<u8>) {
        self.notifier.notify(bytes);
    }

    fn ring_bell(&mut self, cx: &mut Context<Self>) {
        if !cx.global::<Settings>().background.visual_bell {
            return;
        }
        self.bell_flash = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(BELL_FLASH).await;
            let _ = this.update(cx, |this, cx| {
                this.bell_flash = None;
                cx.notify();
            });
        }));
        cx.emit(TerminalEvent::Bell);
        cx.notify();
    }

    fn handle_event(&mut self, event: TermEvent, cx: &mut Context<Self>) {
        match event {
            TermEvent::Wakeup => {
                // Output also means a command may have started or exited
                // without a key press, e.g. ssh failing to connect.
                self.schedule_session_check(cx);
                cx.notify();
            }
            TermEvent::Title(title) => {
                self.title = title.into();
                cx.emit(TerminalEvent::TitleChanged);
            }
            TermEvent::ResetTitle => {
                self.title = DEFAULT_TITLE.into();
                cx.emit(TerminalEvent::TitleChanged);
            }
            TermEvent::ClipboardStore(_, text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            TermEvent::ColorRequest(index, format) => {
                let color = self.term.lock().colors()[index].unwrap_or_else(|| default_rgb(index));
                self.write(format(color).into_bytes());
            }
            TermEvent::PtyWrite(text) => self.write(text.into_bytes()),
            TermEvent::TextAreaSizeRequest(format) => {
                self.write(format(self.window_size()).into_bytes());
            }
            TermEvent::Bell => self.ring_bell(cx),
            TermEvent::ChildExit(_) | TermEvent::Exit => cx.emit(TerminalEvent::Exited),
            _ => {}
        }
    }

    fn layout(&mut self, bounds: Bounds<Pixels>, cell: Size<Pixels>, cx: &mut Context<Self>) {
        self.bounds = bounds;
        self.cell = cell;
        // `bounds` is already a whole number of cells (see `center_grid`); the
        // epsilon keeps float rounding from dropping the last column or row.
        let columns = (bounds.size.width / cell.width + 0.01).floor().max(2.) as usize;
        let lines = (bounds.size.height / cell.height + 0.01).floor().max(1.) as usize;
        if (columns, lines) == (self.columns, self.lines) {
            return;
        }
        self.columns = columns;
        self.lines = lines;
        self.term.lock().resize(TermSize::new(columns, lines));
        let _ = self.notifier.0.send(Msg::Resize(self.window_size()));
        cx.emit(TerminalEvent::GridResized);
    }

    fn input(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        {
            let mut term = self.term.lock();
            term.scroll_display(Scroll::Bottom);
            term.selection = None;
        }
        self.cursor_on = true;
        self.schedule_session_check(cx);
        self.write(bytes);
        cx.notify();
    }

    fn grid_point(&self, position: Point<Pixels>) -> (GridPoint, Side) {
        let x = f32::from(position.x - self.bounds.origin.x).max(0.);
        let y = f32::from(position.y - self.bounds.origin.y).max(0.);
        let cell_width = f32::from(self.cell.width);
        let col = ((x / cell_width) as usize).min(self.columns.saturating_sub(1));
        let line = ((y / f32::from(self.cell.height)) as usize).min(self.lines.saturating_sub(1));
        let side = if x % cell_width > cell_width / 2. {
            Side::Right
        } else {
            Side::Left
        };
        let offset = self.term.lock().grid().display_offset();
        let point = viewport_to_point(
            offset,
            alacritty_terminal::index::Point::new(line, Column(col)),
        );
        (point, side)
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let mode = *self.term.lock().mode();
        if let Some(bytes) = key_to_bytes(&event.keystroke, mode) {
            cx.stop_propagation();
            self.input(bytes, cx);
        }
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        let (point, side) = self.grid_point(event.position);
        let kind = match event.click_count {
            2 => SelectionType::Semantic,
            3.. => SelectionType::Lines,
            _ => SelectionType::Simple,
        };
        self.term.lock().selection = Some(Selection::new(kind, point, side));
        self.drag_position = event.position;
        self.drag_scroll = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(DRAG_SCROLL_INTERVAL).await;
                if this
                    .update(cx, |this, cx| this.drag_scroll_tick(cx))
                    .is_err()
                {
                    break;
                }
            }
        }));
        cx.notify();
    }

    /// A click that did not drag out a selection opens the link under it.
    fn mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.click_count != 1 || self.selected_text().is_some() {
            return;
        }
        let (point, _) = self.grid_point(event.position);
        let term = self.term.lock();
        let link = visible_links(&term, &mut self.link_regex)
            .into_iter()
            .find(|link| link.contains(&point));
        if let Some(link) = link {
            cx.open_url(&term.bounds_to_string(*link.start(), *link.end()));
        }
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let (point, _) = self.grid_point(event.position);
        let hovering_link = self.links.iter().any(|link| link.contains(&point));
        if hovering_link != self.hovering_link {
            self.hovering_link = hovering_link;
            cx.notify();
        }
    }

    fn extend_selection(&self, position: Point<Pixels>) {
        let (point, side) = self.grid_point(position);
        if let Some(selection) = self.term.lock().selection.as_mut() {
            selection.update(point, side);
        }
    }

    /// Lines to scroll per tick: positive above the grid, negative below it, and faster the
    /// further the pointer is from the edge.
    fn drag_scroll_lines(&self) -> i32 {
        let y = self.drag_position.y;
        let (beyond, direction) = if y < self.bounds.top() {
            (self.bounds.top() - y, 1)
        } else if y > self.bounds.bottom() {
            (y - self.bounds.bottom(), -1)
        } else {
            return 0;
        };
        direction * (1 + (beyond / self.cell.height) as i32)
    }

    fn drag_scroll_tick(&mut self, cx: &mut Context<Self>) {
        let lines = self.drag_scroll_lines();
        if lines == 0 {
            return;
        }
        self.term.lock().scroll_display(Scroll::Delta(lines));
        self.extend_selection(self.drag_position);
        cx.notify();
    }

    /// Called for moves anywhere in the window, so a drag keeps extending the selection
    /// after the pointer leaves the terminal.
    /// Like other X11 and Wayland apps, a finished selection becomes the primary selection
    /// that middle click and Shift+Insert paste.
    fn end_drag(&mut self, cx: &mut Context<Self>) {
        if self.drag_scroll.take().is_some()
            && let Some(text) = self.selected_text()
        {
            cx.write_to_primary(ClipboardItem::new_string(text));
        }
    }

    fn drag_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if self.drag_scroll.is_none() || event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        self.drag_position = event.position;
        self.extend_selection(event.position);
        cx.notify();
    }

    fn scroll_wheel(&mut self, event: &ScrollEvent, _: &mut Window, cx: &mut Context<Self>) {
        let line_height = self.cell.height;
        let delta = f32::from(event.delta.pixel_delta(line_height).y) / f32::from(line_height);
        let delta = delta + self.scroll_remainder;
        let lines = delta.trunc() as i32;
        self.scroll_remainder = delta - lines as f32;
        if lines == 0 {
            return;
        }
        let mut term = self.term.lock();
        // Full-screen apps like less or vim get arrow keys instead of scrollback.
        if term
            .mode()
            .contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL)
        {
            drop(term);
            let key: &[u8] = if lines > 0 { b"\x1bOA" } else { b"\x1bOB" };
            self.write(key.repeat(lines.unsigned_abs() as usize));
        } else {
            term.scroll_display(Scroll::Delta(lines));
            drop(term);
            if self.drag_scroll.is_some() {
                self.extend_selection(event.position);
            }
        }
        cx.notify();
    }

    fn scroll_display(&mut self, scroll: Scroll, cx: &mut Context<Self>) {
        self.term.lock().scroll_display(scroll);
        cx.notify();
    }

    fn scroll_line_down(&mut self, _: &ScrollLineDown, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_display(Scroll::Delta(-1), cx);
    }

    fn scroll_line_up(&mut self, _: &ScrollLineUp, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_display(Scroll::Delta(1), cx);
    }

    fn scroll_page_down(&mut self, _: &ScrollPageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_display(Scroll::PageDown, cx);
    }

    fn scroll_page_up(&mut self, _: &ScrollPageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_display(Scroll::PageUp, cx);
    }

    fn scroll_to_bottom(&mut self, _: &ScrollToBottom, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_display(Scroll::Bottom, cx);
    }

    fn scroll_to_top(&mut self, _: &ScrollToTop, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_display(Scroll::Top, cx);
    }

    /// A plain click leaves an empty selection behind, which counts as none.
    fn selected_text(&self) -> Option<String> {
        self.term
            .lock()
            .selection_to_string()
            .filter(|text| !text.is_empty())
    }

    fn copy_selection(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
        self.term.lock().selection = None;
        cx.notify();
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        self.copy_selection(cx);
    }

    fn paste_text(&mut self, text: String, cx: &mut Context<Self>) {
        let bracketed = self.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
        let text = if bracketed {
            // Stripping ESC stops pasted text from ending the bracketed paste early.
            format!("\x1b[200~{}\x1b[201~", text.replace('\x1b', ""))
        } else {
            text.replace("\r\n", "\r").replace('\n', "\r")
        };
        self.input(text.into_bytes(), cx);
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.paste_text(text, cx);
        }
    }

    fn paste_primary_selection(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_primary().and_then(|item| item.text()) {
            self.paste_text(text, cx);
        }
    }

    fn paste_primary(&mut self, _: &PastePrimary, _: &mut Window, cx: &mut Context<Self>) {
        self.paste_primary_selection(cx);
    }

    fn middle_click(&mut self, _: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        self.paste_primary_selection(cx);
    }

    fn right_click(&mut self, _: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.copy_selection(cx);
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = cx.global::<Settings>();
        let font = font(settings.font.family.clone());
        let font_size = px(settings.font.size);
        let cell = cell_size(window.text_system(), settings);
        let focused = self.focus_handle.is_focused(window);
        let cursor_on = self.cursor_on;
        let flash = self.bell_flash.is_some();
        let opacity = settings.opacity();
        let zebra = settings.background.zebra;
        let background_override = match self.session {
            SessionKind::Local => None,
            SessionKind::Container => Some(rgb_from_u32(CONTAINER_BACKGROUND)),
            SessionKind::Root => Some(rgb_from_u32(ROOT_BACKGROUND)),
            SessionKind::Ssh => Some(rgb_from_u32(SSH_BACKGROUND)),
        };
        // A program may have changed the default background (OSC 11).
        let program_background = self.term.lock().colors()[NamedColor::Background as usize];
        let background = background_override
            .or(program_background)
            .unwrap_or(rgb_from_u32(BACKGROUND));
        let view = cx.entity();
        let paint_view = view.clone();
        let term = self.term.clone();
        let links = visible_links(&term.lock(), &mut self.link_regex);
        self.links = links.clone();
        let cursor = if self.hovering_link {
            CursorStyle::PointingHand
        } else {
            CursorStyle::IBeam
        };

        div()
            .id("terminal")
            .key_context("Terminal")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::paste_primary))
            .on_action(cx.listener(Self::scroll_line_down))
            .on_action(cx.listener(Self::scroll_line_up))
            .on_action(cx.listener(Self::scroll_page_down))
            .on_action(cx.listener(Self::scroll_page_up))
            .on_action(cx.listener(Self::scroll_to_bottom))
            .on_action(cx.listener(Self::scroll_to_top))
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::middle_click))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::right_click))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_scroll(cx.listener(Self::scroll_wheel))
            .cursor(cursor)
            .flex_1()
            .size_full()
            .p_1()
            .bg(to_hsla(background).alpha(opacity))
            .child(
                canvas(
                    move |bounds, _, cx| {
                        let grid = center_grid(bounds, cell);
                        view.update(cx, |this, cx| this.layout(grid, cell, cx));
                        grid
                    },
                    move |bounds, grid, window, cx| {
                        let frame = build_frame(
                            &term.lock(),
                            &links,
                            &font,
                            focused,
                            cursor_on,
                            background_override,
                            opacity,
                            zebra,
                        );
                        paint_frame(frame, grid, cell, font_size, focused, window, cx);
                        if flash {
                            window.paint_quad(fill(bounds, hsla(0., 0., 1., 0.15)));
                        }
                        let move_view = paint_view.clone();
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                            if phase == DispatchPhase::Bubble {
                                move_view.update(cx, |this, cx| this.drag_move(event, cx));
                            }
                        });
                        let up_view = paint_view.clone();
                        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                            if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                                up_view.update(cx, |this, cx| this.end_drag(cx));
                            }
                        });
                    },
                )
                .size_full(),
            )
    }
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::test::TermSize;
    use alacritty_terminal::term::{Config, Term};
    use alacritty_terminal::vte::ansi::Processor;

    use super::{
        Keystroke, LINK_PATTERN, Modifiers, RegexSearch, Rgb, TermMode, default_rgb, key_to_bytes,
        visible_links,
    };

    fn links(text: &str) -> Vec<String> {
        let mut term = Term::new(Config::default(), &TermSize::new(80, 5), VoidListener);
        Processor::<alacritty_terminal::vte::ansi::StdSyncHandler>::new()
            .advance(&mut term, text.as_bytes());
        let mut regex = RegexSearch::new(LINK_PATTERN).expect("link pattern is valid");
        visible_links(&term, &mut regex)
            .into_iter()
            .map(|link| term.bounds_to_string(*link.start(), *link.end()))
            .collect()
    }

    #[test]
    fn detects_links() {
        assert_eq!(
            links("see https://example.com/a?b=1. or (http://x.org)\r\nftp://no"),
            ["https://example.com/a?b=1", "http://x.org"]
        );
        assert_eq!(
            links("cd /home/user/dir ~/src ./a/b ../c\r\nls /etc"),
            Vec::<String>::new()
        );
    }

    fn keystroke(key: &str, key_char: Option<&str>, modifiers: Modifiers) -> Keystroke {
        Keystroke {
            modifiers,
            key: key.into(),
            key_char: key_char.map(Into::into),
        }
    }

    fn bytes(keystroke: Keystroke, mode: TermMode) -> Vec<u8> {
        key_to_bytes(&keystroke, mode).expect("key produces input")
    }

    #[test]
    fn printable_and_control_keys() {
        let none = Modifiers::default();
        let ctrl = Modifiers::control();
        let alt = Modifiers::alt();
        assert_eq!(bytes(keystroke("a", Some("a"), none), TermMode::NONE), b"a");
        assert_eq!(bytes(keystroke("c", None, ctrl), TermMode::NONE), b"\x03");
        assert_eq!(
            bytes(keystroke("x", Some("x"), alt), TermMode::NONE),
            b"\x1bx"
        );
        assert_eq!(bytes(keystroke("enter", None, none), TermMode::NONE), b"\r");
        assert_eq!(
            bytes(keystroke("backspace", None, none), TermMode::NONE),
            b"\x7f"
        );
        assert_eq!(
            bytes(keystroke("tab", None, Modifiers::shift()), TermMode::NONE),
            b"\x1b[Z"
        );
    }

    #[test]
    fn cursor_keys_follow_mode_and_modifiers() {
        let none = Modifiers::default();
        assert_eq!(
            bytes(keystroke("up", None, none), TermMode::NONE),
            b"\x1b[A"
        );
        assert_eq!(
            bytes(keystroke("up", None, none), TermMode::APP_CURSOR),
            b"\x1bOA"
        );
        let ctrl = Modifiers::control();
        assert_eq!(
            bytes(keystroke("left", None, ctrl), TermMode::NONE),
            b"\x1b[1;5D"
        );
        assert_eq!(
            bytes(keystroke("pagedown", None, none), TermMode::NONE),
            b"\x1b[6~"
        );
        assert_eq!(
            bytes(keystroke("f1", None, none), TermMode::NONE),
            b"\x1bOP"
        );
    }

    #[test]
    fn palette_covers_cube_and_grayscale() {
        assert_eq!(default_rgb(16), Rgb { r: 0, g: 0, b: 0 });
        assert_eq!(
            default_rgb(231),
            Rgb {
                r: 255,
                g: 255,
                b: 255
            }
        );
        assert_eq!(default_rgb(232), Rgb { r: 8, g: 8, b: 8 });
        assert_eq!(
            default_rgb(255),
            Rgb {
                r: 238,
                g: 238,
                b: 238
            }
        );
    }
}
