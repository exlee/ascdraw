use std::cell::RefCell;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::num::NonZeroU32;
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;
use serde::Deserialize;
use serde_json::{Value, json};
use skia_safe::{
    AlphaType, Canvas, Color, ColorType, Font, FontHinting, FontMgr, FontStyle, ImageInfo, Paint,
    PixelGeometry, Rect, SurfaceProps, SurfacePropsFlags, font::Edging, surfaces,
};
use softbuffer::{Context as SoftContext, Surface};
use unicode_width::UnicodeWidthChar;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, Event, Ime, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use winit::window::WindowLevel;
use winit::window::{Window, WindowAttributes};

const PADDING: usize = 12;
const FALLBACK_BG: Rgb = Rgb::new(0x1e, 0x1e, 0x2e);
const FALLBACK_FG: Rgb = Rgb::new(0xdd, 0xdd, 0xdd);

#[derive(Parser, Debug)]
struct Args {
    file: Option<String>,
    #[arg(long, default_value = "kak")]
    kak_bin: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
struct AppConfig {
    font_family: String,
    font_size: f32,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            font_family: "SF Mono".to_string(),
            font_size: 15.0,
        }
    }
}

#[derive(Debug)]
enum AppEvent {
    Rpc(RpcNotification),
    KakouneExited,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct Face {
    #[serde(default = "default_color")]
    fg: String,
    #[serde(default = "default_color")]
    bg: String,
    #[serde(default = "default_color")]
    underline: String,
    #[serde(default)]
    attributes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Atom {
    face: Face,
    contents: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
struct Coord {
    line: usize,
    column: usize,
}

#[derive(Debug)]
enum RpcNotification {
    Draw {
        lines: Vec<Vec<Atom>>,
        cursor_pos: Coord,
        default_face: Face,
        padding_face: Face,
        widget_columns: usize,
    },
    DrawStatus {
        prompt: Vec<Atom>,
        content: Vec<Atom>,
        cursor_pos: isize,
        mode_line: Vec<Atom>,
        default_face: Face,
        style: String,
    },
    Refresh {
        force: bool,
    },
    SetUiOptions {
        options: serde_json::Map<String, Value>,
    },
    MenuShow,
    MenuSelect,
    MenuHide,
    InfoShow,
    InfoHide,
}

#[derive(Debug, Deserialize)]
struct RpcEnvelope {
    method: String,
    params: Vec<Value>,
}

#[derive(Debug, Clone)]
struct GridState {
    lines: Vec<Vec<Atom>>,
    cursor_pos: Coord,
    default_face: Face,
    padding_face: Face,
    widget_columns: usize,
}

impl Default for GridState {
    fn default() -> Self {
        Self {
            lines: Vec::new(),
            cursor_pos: Coord { line: 0, column: 0 },
            default_face: Face::default(),
            padding_face: Face::default(),
            widget_columns: 0,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct StatusState {
    prompt: Vec<Atom>,
    content: Vec<Atom>,
    cursor_pos: isize,
    mode_line: Vec<Atom>,
    default_face: Face,
    style: String,
}

#[derive(Debug, Clone, Default)]
struct AppState {
    grid: GridState,
    status: Option<StatusState>,
}

#[derive(Clone)]
struct Renderer {
    font_mgr: FontMgr,
    preferred_font_family: String,
    logical_font_size: f32,
    metrics_cache: RefCell<Option<(u64, CellMetrics)>>,
}

#[derive(Clone)]
struct CellMetrics {
    font: Font,
    cell_width: usize,
    cell_height: usize,
    baseline_offset: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rgb {
    r: u8,
    g: u8,
    b: u8,
}

impl Rgb {
    const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    fn to_color(self) -> Color {
        Color::from_rgb(self.r, self.g, self.b)
    }
}

fn default_color() -> String {
    "default".to_string()
}

fn load_config() -> Result<AppConfig> {
    let path = "kakvide.toml";
    match fs::read_to_string(path) {
        Ok(contents) => toml::from_str(&contents)
            .with_context(|| format!("failed to parse {path}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(AppConfig::default()),
        Err(error) => Err(error).with_context(|| format!("failed to read {path}")),
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    let config = load_config()?;

    let event_loop = EventLoop::<AppEvent>::with_user_event().build()?;
    let attrs = WindowAttributes::default()
        .with_title("kakvide")
        .with_window_level(WindowLevel::Normal)
        .with_inner_size(LogicalSize::new(1200.0, 800.0));
    let window = Rc::new(event_loop.create_window(attrs)?);
    let renderer = load_renderer(&config);
    let context = SoftContext::new(window.clone()).map_err(|error| anyhow!(error.to_string()))?;
    let mut surface =
        Surface::new(&context, window.clone()).map_err(|error| anyhow!(error.to_string()))?;
    resize_surface(&mut surface, window.inner_size())?;

    let mut child = spawn_kakoune(&args, event_loop.create_proxy())?;
    let command_tx = spawn_stdin_writer(&mut child)?;

    let mut modifiers = ModifiersState::empty();
    let mut mouse_cell = Coord { line: 0, column: 0 };
    let mut did_force_startup_resize = false;
    let mut state = AppState::default();

    send_resize(&command_tx, &window, &renderer);
    window.request_redraw();

    event_loop.run(move |event, elwt| {
        elwt.set_control_flow(ControlFlow::Wait);

        match event {
            Event::Resumed => {
                send_resize(&command_tx, &window, &renderer);
                window.request_redraw();
            }
            Event::UserEvent(AppEvent::Rpc(notification)) => {
                let should_force_resize =
                    matches!(notification, RpcNotification::Draw { .. }) && !did_force_startup_resize;
                apply_notification(&mut state, notification);
                if should_force_resize {
                    send_resize(&command_tx, &window, &renderer);
                    did_force_startup_resize = true;
                }
                window.request_redraw();
            }
            Event::UserEvent(AppEvent::KakouneExited) => {
                elwt.exit();
            }
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => {
                    let _ = child.kill();
                    elwt.exit();
                }
                WindowEvent::Resized(size) => {
                    if let Err(error) = resize_surface(&mut surface, size) {
                        eprintln!("surface resize failed: {error:#}");
                    }
                    send_resize(&command_tx, &window, &renderer);
                    window.request_redraw();
                }
                WindowEvent::RedrawRequested => {
                    if let Err(error) = render(&window, &mut surface, &state, &renderer) {
                        eprintln!("render failed: {error:#}");
                        let _ = child.kill();
                        elwt.exit();
                    }
                }
                WindowEvent::ModifiersChanged(new_modifiers) => {
                    modifiers = new_modifiers.state();
                }
                WindowEvent::Ime(Ime::Commit(text)) => {
                    if !text.is_empty()
                        && !modifiers.control_key()
                        && !modifiers.alt_key()
                        && !modifiers.super_key()
                    {
                        send_keys(&command_tx, &[text.to_string()]);
                    }
                }
                WindowEvent::KeyboardInput { event, .. } => {
                    if event.state == ElementState::Pressed {
                        if let Some(keys) = key_event_to_kak(&event, modifiers) {
                            send_keys(&command_tx, &[keys]);
                        }
                    }
                }
                WindowEvent::CursorMoved { position, .. } => {
                    mouse_cell = pointer_position_to_coord(position.x, position.y, &renderer, &window);
                    send_mouse_move(&command_tx, mouse_cell);
                }
                WindowEvent::MouseInput { state, button, .. } => {
                    match state {
                        ElementState::Pressed => send_mouse_button(&command_tx, "mouse_press", button, mouse_cell),
                        ElementState::Released => {
                            send_mouse_button(&command_tx, "mouse_release", button, mouse_cell)
                        }
                    }
                }
                WindowEvent::MouseWheel { delta, .. } => {
                    if let Some(amount) = scroll_delta_to_kak(delta) {
                        send_scroll(&command_tx, amount, mouse_cell);
                    }
                }
                WindowEvent::ScaleFactorChanged { .. } => {
                    send_resize(&command_tx, &window, &renderer);
                    window.request_redraw();
                }
                _ => {}
            },
            _ => {}
        }
    })?;

    Ok(())
}

fn spawn_kakoune(args: &Args, proxy: EventLoopProxy<AppEvent>) -> Result<Child> {
    let mut command = Command::new(&args.kak_bin);
    command.arg("-ui").arg("json");
    if let Some(file) = &args.file {
        command.arg(file);
    }
    command.stdin(Stdio::piped());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command
        .spawn()
        .with_context(|| format!("failed to start {}", args.kak_bin))?;

    let stdout = child.stdout.take().context("missing kakoune stdout pipe")?;
    let stderr = child.stderr.take().context("missing kakoune stderr pipe")?;

    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            match line {
                Ok(line) => match parse_notification(&line) {
                    Ok(notification) => {
                        let _ = proxy.send_event(AppEvent::Rpc(notification));
                    }
                    Err(error) => eprintln!("json ui parse error: {error:#}\nline: {line}"),
                },
                Err(error) => {
                    eprintln!("stdout read error: {error:#}");
                    break;
                }
            }
        }
        let _ = proxy.send_event(AppEvent::KakouneExited);
    });

    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            match line {
                Ok(line) => eprintln!("kak stderr: {line}"),
                Err(error) => {
                    eprintln!("stderr read error: {error:#}");
                    break;
                }
            }
        }
    });

    Ok(child)
}

fn spawn_stdin_writer(child: &mut Child) -> Result<Sender<String>> {
    let stdin = child.stdin.take().context("missing kakoune stdin pipe")?;
    let (tx, rx): (Sender<String>, Receiver<String>) = mpsc::channel();

    thread::spawn(move || {
        let mut stdin = stdin;
        while let Ok(line) = rx.recv() {
            if stdin.write_all(line.as_bytes()).is_err() {
                break;
            }
            if stdin.write_all(b"\n").is_err() {
                break;
            }
            if stdin.flush().is_err() {
                break;
            }
        }
    });

    Ok(tx)
}

fn parse_notification(line: &str) -> Result<RpcNotification> {
    let envelope: RpcEnvelope = serde_json::from_str(line)?;
    match envelope.method.as_str() {
        "draw" => {
            let (lines, cursor_pos, default_face, padding_face, widget_columns): (
                Vec<Vec<Atom>>,
                Coord,
                Face,
                Face,
                usize,
            ) = deserialize_params(envelope.params)?;
            Ok(RpcNotification::Draw {
                lines,
                cursor_pos,
                default_face,
                padding_face,
                widget_columns,
            })
        }
        "draw_status" => {
            let (prompt, content, cursor_pos, mode_line, default_face, style): (
                Vec<Atom>,
                Vec<Atom>,
                isize,
                Vec<Atom>,
                Face,
                String,
            ) = deserialize_params(envelope.params)?;
            Ok(RpcNotification::DrawStatus {
                prompt,
                content,
                cursor_pos,
                mode_line,
                default_face,
                style,
            })
        }
        "refresh" => {
            let (force,): (bool,) = deserialize_params(envelope.params)?;
            Ok(RpcNotification::Refresh { force })
        }
        "set_ui_options" => {
            let (options,): (serde_json::Map<String, Value>,) =
                deserialize_params(envelope.params)?;
            Ok(RpcNotification::SetUiOptions { options })
        }
        "menu_show" => Ok(RpcNotification::MenuShow),
        "menu_select" => Ok(RpcNotification::MenuSelect),
        "menu_hide" => Ok(RpcNotification::MenuHide),
        "info_show" => Ok(RpcNotification::InfoShow),
        "info_hide" => Ok(RpcNotification::InfoHide),
        other => bail!("unsupported rpc method {other}"),
    }
}

fn deserialize_params<T>(params: Vec<Value>) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    Ok(serde_json::from_value(Value::Array(params))?)
}

fn apply_notification(state: &mut AppState, notification: RpcNotification) {
    match notification {
        RpcNotification::Draw {
            lines,
            cursor_pos,
            default_face,
            padding_face,
            widget_columns,
        } => {
            state.grid = GridState {
                lines,
                cursor_pos,
                default_face,
                padding_face,
                widget_columns,
            };
        }
        RpcNotification::DrawStatus {
            prompt,
            content,
            cursor_pos,
            mode_line,
            default_face,
            style,
        } => {
            state.status = Some(StatusState {
                prompt,
                content,
                cursor_pos,
                mode_line,
                default_face,
                style,
            });
        }
        RpcNotification::Refresh { force } => {
            let _ = force;
        }
        RpcNotification::SetUiOptions { options } => {
            let _ = options;
        }
        RpcNotification::MenuShow
        | RpcNotification::MenuSelect
        | RpcNotification::MenuHide
        | RpcNotification::InfoShow
        | RpcNotification::InfoHide => {}
    }
}

fn render(
    window: &Window,
    surface: &mut Surface<Rc<Window>, Rc<Window>>,
    state: &AppState,
    renderer: &Renderer,
) -> Result<()> {
    let size = window.inner_size();
    let width = size.width.max(1) as usize;
    let height = size.height.max(1) as usize;
    let metrics = renderer.metrics(window.scale_factor());

    let mut buffer = surface
        .buffer_mut()
        .map_err(|error| anyhow!(error.to_string()))?;
    let pixels = unsafe { buffer_as_u8_mut(buffer.as_mut()) };

    let image_info = ImageInfo::new(
        (width as i32, height as i32),
        ColorType::BGRA8888,
        AlphaType::Premul,
        None,
    );
    let props = SurfaceProps::new(
        SurfacePropsFlags::USE_DEVICE_INDEPENDENT_FONTS,
        PixelGeometry::Unknown,
    );
    let mut skia_surface = surfaces::wrap_pixels(&image_info, pixels, width * 4, Some(&props))
        .context("failed to wrap Skia surface around window buffer")?;
    let canvas = skia_surface.canvas();

    render_canvas(canvas, width, height, state, &metrics);

    buffer
        .present()
        .map_err(|error| anyhow!(error.to_string()))?;
    Ok(())
}

fn render_canvas(
    canvas: &Canvas,
    width: usize,
    height: usize,
    state: &AppState,
    metrics: &CellMetrics,
) {
    let bg = resolve_color(&state.grid.default_face.bg, FALLBACK_BG).to_color();
    canvas.clear(bg);

    let cols = width.saturating_sub(PADDING * 2) / metrics.cell_width.max(1);
    let rows = height.saturating_sub(PADDING * 2) / metrics.cell_height.max(1);
    let status_rows = usize::from(state.status.is_some());
    let grid_rows = rows.saturating_sub(status_rows);

    for (row_index, line) in state.grid.lines.iter().take(grid_rows).enumerate() {
        render_line(
            canvas,
            row_index,
            line,
            &state.grid.default_face,
            cols,
            metrics,
        );
    }

    if let Some(status) = &state.status {
        render_status(canvas, rows, cols, status, metrics);
    }

}

fn render_status(
    canvas: &Canvas,
    total_rows: usize,
    cols: usize,
    status: &StatusState,
    metrics: &CellMetrics,
) {
    let row = total_rows.saturating_sub(1);
    fill_line_background(canvas, row, cols, &status.default_face, metrics);

    let mut prompt_line = status.prompt.clone();
    prompt_line.extend(status.content.clone());

    let mode_width = line_display_width(&status.mode_line);
    let right_start = cols.saturating_sub(mode_width);
    let prompt_limit = if prompt_line.is_empty() { cols } else { right_start };

    if !prompt_line.is_empty() {
        render_line_at(
            canvas,
            row,
            0,
            &prompt_line,
            &status.default_face,
            prompt_limit,
            metrics,
        );
    }

    if !status.mode_line.is_empty() {
        render_line_at(
            canvas,
            row,
            right_start,
            &status.mode_line,
            &status.default_face,
            cols,
            metrics,
        );
    }
}

fn render_line(
    canvas: &Canvas,
    row: usize,
    line: &[Atom],
    default_face: &Face,
    max_columns: usize,
    metrics: &CellMetrics,
) {
    render_line_at(canvas, row, 0, line, default_face, max_columns, metrics);
}

fn render_line_at(
    canvas: &Canvas,
    row: usize,
    start_column: usize,
    line: &[Atom],
    default_face: &Face,
    max_columns: usize,
    metrics: &CellMetrics,
) {
    let top = PADDING + row * metrics.cell_height;
    let mut column = start_column;
    let mut bg_paint = Paint::default();
    bg_paint.set_anti_alias(false);
    let mut fg_paint = Paint::default();
    fg_paint.set_anti_alias(true);

    for atom in line {
        let fg = resolve_face_color(&atom.face.fg, &default_face.fg, FALLBACK_FG);
        let bg = resolve_face_color(&atom.face.bg, &default_face.bg, FALLBACK_BG);
        let _ = (&atom.face.underline, &atom.face.attributes);
        bg_paint.set_color(bg.to_color());
        fg_paint.set_color(fg.to_color());

        let atom_width = atom_display_width(&atom.contents);
        if atom_width == 0 {
            continue;
        }

        let atom_start = column;
        let atom_width = atom_width.min(max_columns.saturating_sub(atom_start));
        if atom_width == 0 {
            return;
        }
        if atom_width > 0 {
            fill_cells(canvas, atom_start, top, atom_width, metrics, &bg_paint);
        }

        for ch in atom.contents.chars() {
            if ch == '\n' {
                continue;
            }

            let span = UnicodeWidthChar::width(ch).unwrap_or(1).max(1);
            draw_glyph(canvas, column, top, ch, metrics, &fg_paint);
            column += span;
            if column >= max_columns {
                return;
            }
        }
    }
}

fn atom_display_width(contents: &str) -> usize {
    contents
        .chars()
        .filter(|&ch| ch != '\n')
        .map(|ch| UnicodeWidthChar::width(ch).unwrap_or(1).max(1))
        .sum()
}

fn line_display_width(line: &[Atom]) -> usize {
    line
        .iter()
        .map(|atom| atom_display_width(&atom.contents))
        .sum()
}

fn fill_line_background(
    canvas: &Canvas,
    row: usize,
    cols: usize,
    default_face: &Face,
    metrics: &CellMetrics,
) {
    let bg = resolve_face_color(&default_face.bg, &default_face.bg, FALLBACK_BG).to_color();
    let mut paint = Paint::default();
    paint.set_anti_alias(false).set_color(bg);
    fill_cells(canvas, 0, PADDING + row * metrics.cell_height, cols, metrics, &paint);
}

fn fill_cells(
    canvas: &Canvas,
    column: usize,
    top: usize,
    width_in_cells: usize,
    metrics: &CellMetrics,
    paint: &Paint,
) {
    let left = PADDING + column * metrics.cell_width;
    let rect = Rect::from_xywh(
        left as f32,
        top as f32,
        (metrics.cell_width * width_in_cells) as f32,
        metrics.cell_height as f32,
    );
    canvas.draw_rect(rect, paint);
}

fn draw_glyph(
    canvas: &Canvas,
    column: usize,
    top: usize,
    ch: char,
    metrics: &CellMetrics,
    paint: &Paint,
) {
    if ch.is_control() {
        return;
    }

    let left = PADDING + column * metrics.cell_width;
    let baseline = top as f32 + metrics.baseline_offset;
    let mut utf8 = [0; 4];
    let text = ch.encode_utf8(&mut utf8);
    canvas.draw_str(
        text,
        (left as f32, baseline),
        &metrics.font,
        paint,
    );
}

fn send_resize(tx: &Sender<String>, window: &Window, renderer: &Renderer) {
    let size = window.inner_size();
    let metrics = renderer.metrics(window.scale_factor());
    let cols = ((size.width as usize).saturating_sub(PADDING * 2) / metrics.cell_width).max(1);
    let rows = ((size.height as usize).saturating_sub(PADDING * 2) / metrics.cell_height).max(1);
    send_rpc(tx, "resize", json!([rows, cols]));
}

fn send_keys(tx: &Sender<String>, keys: &[String]) {
    send_rpc(
        tx,
        "keys",
        Value::Array(keys.iter().cloned().map(Value::String).collect()),
    );
}

fn send_mouse_move(tx: &Sender<String>, coord: Coord) {
    send_rpc(tx, "mouse_move", json!([coord.line, coord.column]));
}

fn send_mouse_button(tx: &Sender<String>, method: &str, button: MouseButton, coord: Coord) {
    if let Some(button) = mouse_button_to_kak(button) {
        send_rpc(tx, method, json!([button, coord.line, coord.column]));
    }
}

fn send_scroll(tx: &Sender<String>, amount: i32, coord: Coord) {
    send_rpc(tx, "scroll", json!([amount, coord.line, coord.column]));
}

fn send_rpc(tx: &Sender<String>, method: &str, params: Value) {
    let message = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    });
    let _ = tx.send(message.to_string());
}

fn pointer_position_to_coord(x: f64, y: f64, renderer: &Renderer, window: &Window) -> Coord {
    let metrics = renderer.metrics(window.scale_factor());
    let column = ((x - PADDING as f64).max(0.0) / metrics.cell_width.max(1) as f64).floor() as usize;
    let line = ((y - PADDING as f64).max(0.0) / metrics.cell_height.max(1) as f64).floor() as usize;
    Coord { line, column }
}

fn mouse_button_to_kak(button: MouseButton) -> Option<&'static str> {
    match button {
        MouseButton::Left => Some("left"),
        MouseButton::Right => Some("right"),
        MouseButton::Middle => Some("middle"),
        _ => None,
    }
}

fn scroll_delta_to_kak(delta: MouseScrollDelta) -> Option<i32> {
    let amount = match delta {
        MouseScrollDelta::LineDelta(x, y) => dominant_scroll_component(x as f64, y as f64),
        MouseScrollDelta::PixelDelta(position) => dominant_scroll_component(position.x, position.y),
    };

    if amount == 0 {
        None
    } else {
        Some(amount)
    }
}

fn dominant_scroll_component(x: f64, y: f64) -> i32 {
    let dominant = if y.abs() >= x.abs() { y } else { x };
    if dominant > 0.0 {
        -dominant.abs().ceil() as i32
    } else if dominant < 0.0 {
        dominant.abs().ceil() as i32
    } else {
        0
    }
}

fn key_event_to_kak(event: &KeyEvent, modifiers: ModifiersState) -> Option<String> {
    match &event.logical_key {
        Key::Named(named) => named_key_to_kak(*named, modifiers),
        Key::Character(text) => {
            if modifiers.control_key() || modifiers.alt_key() {
                modified_character_key_to_kak(&event.key_without_modifiers(), modifiers)
            } else {
                Some(text.to_string())
            }
        }
        _ => None,
    }
}

fn modified_character_key_to_kak(key: &Key, modifiers: ModifiersState) -> Option<String> {
    let Key::Character(text) = key else {
        return None;
    };

    let mut chars = text.chars();
    let ch = chars.next()?;
    if chars.next().is_some() {
        return None;
    }

    let mut result = String::from("<");
    if modifiers.shift_key() {
        result.push_str("s-");
    }
    if modifiers.alt_key() {
        result.push_str("a-");
    }
    if modifiers.control_key() {
        result.push_str("c-");
    }
    result.push(ch.to_ascii_lowercase());
    result.push('>');
    Some(result)
}

fn named_key_to_kak(key: NamedKey, modifiers: ModifiersState) -> Option<String> {
    let base = match key {
        NamedKey::Enter => "ret",
        NamedKey::Tab => "tab",
        NamedKey::Space => "space",
        NamedKey::Escape => "esc",
        NamedKey::ArrowUp => "up",
        NamedKey::ArrowDown => "down",
        NamedKey::ArrowLeft => "left",
        NamedKey::ArrowRight => "right",
        NamedKey::Backspace => "backspace",
        NamedKey::Delete => "del",
        NamedKey::Home => "home",
        NamedKey::End => "end",
        NamedKey::PageUp => "pageup",
        NamedKey::PageDown => "pagedown",
        _ => return None,
    };

    if modifiers == ModifiersState::empty() {
        return Some(format!("<{base}>"));
    }

    let mut result = String::from("<");
    if modifiers.shift_key() {
        result.push_str("s-");
    }
    if modifiers.alt_key() {
        result.push_str("a-");
    }
    if modifiers.control_key() {
        result.push_str("c-");
    }
    result.push_str(base);
    result.push('>');
    Some(result)
}

fn resolve_face_color(color: &str, inherited: &str, fallback: Rgb) -> Rgb {
    if color == "default" {
        resolve_color(inherited, fallback)
    } else {
        resolve_color(color, fallback)
    }
}

fn resolve_color(color: &str, fallback: Rgb) -> Rgb {
    if color == "default" {
        return fallback;
    }

    if let Some(rgb) = parse_prefixed_color(color) {
        return rgb;
    }

    match color {
        "black" => Rgb::new(0x00, 0x00, 0x00),
        "white" => Rgb::new(0xff, 0xff, 0xff),
        "red" => Rgb::new(0xff, 0x55, 0x55),
        "green" => Rgb::new(0x50, 0xfa, 0x7b),
        "yellow" => Rgb::new(0xf1, 0xfa, 0x8c),
        "blue" => Rgb::new(0x62, 0xd6, 0xe8),
        "magenta" => Rgb::new(0xff, 0x79, 0xc6),
        "cyan" => Rgb::new(0x8b, 0xe9, 0xfd),
        _ => fallback,
    }
}

fn parse_prefixed_color(value: &str) -> Option<Rgb> {
    if let Some(rgb) = value.strip_prefix("rgb:").and_then(parse_hex_color) {
        return Some(rgb);
    }
    if let Some(rgb) = value.strip_prefix("rgba:").and_then(parse_rgba_color) {
        return Some(rgb);
    }
    parse_hex_color(value)
}

fn parse_hex_color(value: &str) -> Option<Rgb> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() != 6 {
        return None;
    }

    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Rgb::new(r, g, b))
}

fn parse_rgba_color(value: &str) -> Option<Rgb> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() != 8 {
        return None;
    }

    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Rgb::new(r, g, b))
}

fn load_renderer(config: &AppConfig) -> Renderer {
    Renderer {
        font_mgr: FontMgr::new(),
        preferred_font_family: config.font_family.clone(),
        logical_font_size: config.font_size,
        metrics_cache: RefCell::new(None),
    }
}

impl Renderer {
    fn metrics(&self, scale_factor: f64) -> CellMetrics {
        let cache_key = scale_factor.to_bits();
        if let Some((cached_key, metrics)) = self.metrics_cache.borrow().as_ref() {
            if *cached_key == cache_key {
                return metrics.clone();
            }
        }

        let physical_font_size = (self.logical_font_size as f64 * scale_factor) as f32;
        let typeface = preferred_typeface(&self.font_mgr, &self.preferred_font_family).unwrap_or_else(|| {
            self.font_mgr
                .match_family_style("", FontStyle::normal())
                .expect("expected a fallback system typeface")
        });

        let mut font = Font::new(typeface, physical_font_size);
        font.set_subpixel(true)
            .set_edging(Edging::SubpixelAntiAlias)
            .set_hinting(FontHinting::Full)
            .set_baseline_snap(false)
            .set_linear_metrics(false);

        let (_, metrics) = font.metrics();
        let cell_width = font.measure_str("M", None).0.ceil().max(1.0) as usize;
        let cell_height = (metrics.descent - metrics.ascent + metrics.leading)
            .ceil()
            .max(1.0) as usize;
        let baseline_offset = (-metrics.ascent).ceil();

        let metrics = CellMetrics {
            font,
            cell_width,
            cell_height: cell_height.max(16),
            baseline_offset,
        };
        self.metrics_cache
            .borrow_mut()
            .replace((cache_key, metrics.clone()));
        metrics
    }
}

fn preferred_typeface(font_mgr: &FontMgr, configured_family: &str) -> Option<skia_safe::Typeface> {
    [
        configured_family,
        "SF Mono",
        "Menlo",
        "Monaco",
        "JetBrains Mono",
        "Courier New",
    ]
    .iter()
    .find_map(|family| font_mgr.match_family_style(family, FontStyle::normal()))
}

unsafe fn buffer_as_u8_mut(buffer: &mut [u32]) -> &mut [u8] {
    unsafe {
        std::slice::from_raw_parts_mut(
            buffer.as_mut_ptr() as *mut u8,
            std::mem::size_of_val(buffer),
        )
    }
}

fn resize_surface(
    surface: &mut Surface<Rc<Window>, Rc<Window>>,
    size: winit::dpi::PhysicalSize<u32>,
) -> Result<()> {
    let width = NonZeroU32::new(size.width.max(1)).expect("width is non-zero");
    let height = NonZeroU32::new(size.height.max(1)).expect("height is non-zero");
    surface
        .resize(width, height)
        .map_err(|error| anyhow!(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_alt_shift_character_keys_from_unmodified_character() {
        let modifiers = ModifiersState::ALT | ModifiersState::SHIFT;
        let key = Key::Character(";".into());
        assert_eq!(
            modified_character_key_to_kak(&key, modifiers).as_deref(),
            Some("<s-a-;>")
        );
    }

    #[test]
    fn formats_ctrl_shift_letter_keys_with_explicit_shift_modifier() {
        let modifiers = ModifiersState::CONTROL | ModifiersState::SHIFT;
        let key = Key::Character("p".into());
        assert_eq!(
            modified_character_key_to_kak(&key, modifiers).as_deref(),
            Some("<s-c-p>")
        );
    }

    #[test]
    fn status_uses_mode_line_and_prompt_rows() {
        let status = StatusState {
            prompt: vec![Atom {
                face: Face::default(),
                contents: ":".into(),
            }],
            content: vec![Atom {
                face: Face::default(),
                contents: "w".into(),
            }],
            cursor_pos: 1,
            mode_line: vec![Atom {
                face: Face::default(),
                contents: "status".into(),
            }],
            default_face: Face::default(),
            style: "status".into(),
        };
        let mut prompt_line = status.prompt.clone();
        prompt_line.extend(status.content.clone());
        assert_eq!(line_display_width(&prompt_line), 2);
        assert_eq!(line_display_width(&status.mode_line), 6);
    }

    #[test]
    fn scroll_delta_maps_wheel_up_to_negative_kak_scroll() {
        assert_eq!(
            scroll_delta_to_kak(MouseScrollDelta::LineDelta(0.0, 1.0)),
            Some(-1)
        );
        assert_eq!(
            scroll_delta_to_kak(MouseScrollDelta::LineDelta(0.0, -2.0)),
            Some(2)
        );
    }

    #[test]
    fn line_display_width_ignores_empty_spacer_atoms() {
        let line = vec![
            Atom {
                face: Face::default(),
                contents: "left".into(),
            },
            Atom {
                face: Face::default(),
                contents: "".into(),
            },
            Atom {
                face: Face::default(),
                contents: "right".into(),
            },
        ];

        assert_eq!(atom_display_width(&line[1].contents), 0);
        assert_eq!(line_display_width(&line), 9);
    }

    #[test]
    fn parses_rgba_colors_by_ignoring_alpha() {
        assert_eq!(
            parse_prefixed_color("rgba:ffffff80"),
            Some(Rgb::new(0xff, 0xff, 0xff))
        );
    }

    #[test]
    fn config_defaults_match_kakvide_toml_shape() {
        let config = AppConfig::default();
        assert_eq!(config.font_family, "SF Mono");
        assert_eq!(config.font_size, 15.0);
    }
}
