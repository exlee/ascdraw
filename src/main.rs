use std::io::{BufRead, BufReader, Write};
use std::num::NonZeroU32;
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use ab_glyph::{Font, FontArc, Glyph, PxScale, ScaleFont, point};
use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;
use fontdb::{Database, Family, Query};
use serde::Deserialize;
use serde_json::{Value, json};
use softbuffer::{Context as SoftContext, Surface};
use unicode_width::UnicodeWidthChar;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, Event, Ime, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
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
    font: FontArc,
    font_size: PxScale,
    cell_width: usize,
    cell_height: usize,
    ascent: f32,
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

    fn pack(self) -> u32 {
        ((self.r as u32) << 16) | ((self.g as u32) << 8) | self.b as u32
    }
}

fn default_color() -> String {
    "default".to_string()
}

fn main() -> Result<()> {
    let args = Args::parse();

    let event_loop = EventLoop::<AppEvent>::with_user_event().build()?;
    let attrs = WindowAttributes::default()
        .with_title("kakvide")
        .with_inner_size(LogicalSize::new(1200.0, 800.0));
    let window = Rc::new(event_loop.create_window(attrs)?);
    let renderer = load_renderer()?;
    let context = SoftContext::new(window.clone()).map_err(|error| anyhow!(error.to_string()))?;
    let mut surface =
        Surface::new(&context, window.clone()).map_err(|error| anyhow!(error.to_string()))?;

    let mut child = spawn_kakoune(&args, event_loop.create_proxy())?;
    let command_tx = spawn_stdin_writer(&mut child)?;

    let mut modifiers = ModifiersState::empty();
    let mut state = AppState::default();

    send_resize(&command_tx, &window, &renderer);
    window.request_redraw();

    event_loop.run(move |event, elwt| {
        elwt.set_control_flow(ControlFlow::Wait);

        match event {
            Event::UserEvent(AppEvent::Rpc(notification)) => {
                apply_notification(&mut state, notification);
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
                    let width = NonZeroU32::new(size.width.max(1)).expect("width is non-zero");
                    let height = NonZeroU32::new(size.height.max(1)).expect("height is non-zero");
                    if let Err(error) = surface.resize(width, height) {
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
                    if !text.is_empty() && !modifiers.control_key() && !modifiers.super_key() {
                        send_keys(&command_tx, &[text.to_string()]);
                    }
                }
                WindowEvent::KeyboardInput { event, .. } => {
                    if event.state == ElementState::Pressed {
                        if let Some(keys) = key_event_to_kak(&event.logical_key, modifiers) {
                            send_keys(&command_tx, &[keys]);
                        }
                    }
                }
                _ => {}
            },
            Event::AboutToWait => {
                window.request_redraw();
            }
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

    surface
        .resize(
            NonZeroU32::new(size.width.max(1)).expect("width is non-zero"),
            NonZeroU32::new(size.height.max(1)).expect("height is non-zero"),
        )
        .map_err(|error| anyhow!(error.to_string()))?;

    let mut buffer = surface
        .buffer_mut()
        .map_err(|error| anyhow!(error.to_string()))?;
    let bg = resolve_color(&state.grid.default_face.bg, FALLBACK_BG);
    buffer.fill(bg.pack());

    let cols = width.saturating_sub(PADDING * 2) / renderer.cell_width;
    let rows = height.saturating_sub(PADDING * 2) / renderer.cell_height;

    for (row_index, line) in state.grid.lines.iter().take(rows).enumerate() {
        render_line(
            &mut buffer,
            width,
            row_index,
            line,
            &state.grid.default_face,
            cols,
            renderer,
        );
    }

    if let Some(status) = &state.status {
        let status_row = rows.saturating_sub(1);
        let mut line = status.prompt.clone();
        line.extend(status.content.clone());
        render_line(
            &mut buffer,
            width,
            status_row,
            &line,
            &status.default_face,
            cols,
            renderer,
        );
    }

    render_cursor(
        &mut buffer,
        width,
        height,
        state.grid.cursor_pos,
        &state.grid.default_face,
        renderer,
    );

    buffer
        .present()
        .map_err(|error| anyhow!(error.to_string()))?;
    Ok(())
}

fn render_line(
    buffer: &mut [u32],
    surface_width: usize,
    row: usize,
    line: &[Atom],
    default_face: &Face,
    max_columns: usize,
    renderer: &Renderer,
) {
    let top = PADDING + row * renderer.cell_height;
    let mut column = 0usize;

    for atom in line {
        let _ = (&atom.face.underline, &atom.face.attributes);
        let fg = resolve_face_color(&atom.face.fg, &default_face.fg, FALLBACK_FG);
        let bg = resolve_face_color(&atom.face.bg, &default_face.bg, FALLBACK_BG);

        for ch in atom.contents.chars() {
            if ch == '\n' {
                continue;
            }
            let cell_span = UnicodeWidthChar::width(ch).unwrap_or(1).max(1);
            for offset in 0..cell_span {
                fill_rect(
                    buffer,
                    surface_width,
                    PADDING + (column + offset) * renderer.cell_width,
                    top,
                    renderer.cell_width,
                    renderer.cell_height,
                    bg.pack(),
                );
            }
            draw_char(
                buffer,
                surface_width,
                PADDING + column * renderer.cell_width,
                top,
                ch,
                fg.pack(),
                renderer,
            );
            column += cell_span;
            if column >= max_columns {
                return;
            }
        }
    }
}

fn render_cursor(
    buffer: &mut [u32],
    surface_width: usize,
    surface_height: usize,
    cursor_pos: Coord,
    default_face: &Face,
    renderer: &Renderer,
) {
    let cursor_x = PADDING + cursor_pos.column * renderer.cell_width;
    let cursor_y = PADDING + cursor_pos.line * renderer.cell_height;
    if cursor_x + renderer.cell_width > surface_width
        || cursor_y + renderer.cell_height > surface_height
    {
        return;
    }

    let color = resolve_face_color(&default_face.fg, &default_face.fg, FALLBACK_FG).pack();
    fill_rect(
        buffer,
        surface_width,
        cursor_x,
        cursor_y + renderer.cell_height.saturating_sub(2),
        renderer.cell_width,
        2,
        color,
    );
}

fn draw_char(
    buffer: &mut [u32],
    surface_width: usize,
    x: usize,
    y: usize,
    ch: char,
    fg: u32,
    renderer: &Renderer,
) {
    if ch.is_control() {
        return;
    }

    let glyph = Glyph {
        id: renderer.font.glyph_id(ch),
        scale: renderer.font_size,
        position: point(x as f32, y as f32 + renderer.ascent),
    };

    let Some(outlined) = renderer.font.outline_glyph(glyph) else {
        return;
    };

    outlined.draw(|gx, gy, coverage| {
        if coverage < 0.2 {
            return;
        }
        let px = x + gx as usize;
        let py = y + gy as usize;
        set_pixel(buffer, surface_width, px, py, blend(fg, coverage));
    });
}

fn fill_rect(
    buffer: &mut [u32],
    surface_width: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    color: u32,
) {
    for row in y..y.saturating_add(height) {
        let start = row.saturating_mul(surface_width).saturating_add(x);
        let end = start.saturating_add(width).min(buffer.len());
        if start >= buffer.len() || start >= end {
            continue;
        }
        buffer[start..end].fill(color);
    }
}

fn set_pixel(buffer: &mut [u32], surface_width: usize, x: usize, y: usize, color: u32) {
    let index = y.saturating_mul(surface_width).saturating_add(x);
    if index < buffer.len() {
        buffer[index] = color;
    }
}

fn send_resize(tx: &Sender<String>, window: &Window, renderer: &Renderer) {
    let size = window.inner_size();
    let cols = ((size.width as usize).saturating_sub(PADDING * 2) / renderer.cell_width).max(1);
    let rows = ((size.height as usize).saturating_sub(PADDING * 2) / renderer.cell_height).max(1);
    send_rpc(tx, "resize", json!([rows, cols]));
}

fn send_keys(tx: &Sender<String>, keys: &[String]) {
    send_rpc(
        tx,
        "keys",
        Value::Array(keys.iter().cloned().map(Value::String).collect()),
    );
}

fn send_rpc(tx: &Sender<String>, method: &str, params: Value) {
    let message = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    });
    let _ = tx.send(message.to_string());
}

fn key_event_to_kak(key: &Key, modifiers: ModifiersState) -> Option<String> {
    match key {
        Key::Named(named) => named_key_to_kak(*named, modifiers),
        Key::Character(text) => {
            if modifiers.control_key() || modifiers.alt_key() {
                let mut chars = text.chars();
                let ch = chars.next()?;
                if chars.next().is_some() {
                    return None;
                }
                let mut prefix = String::from("<");
                if modifiers.control_key() {
                    prefix.push_str("c-");
                }
                if modifiers.alt_key() {
                    prefix.push_str("a-");
                }
                prefix.push(ch);
                prefix.push('>');
                Some(prefix)
            } else {
                None
            }
        }
        _ => None,
    }
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

    if let Some(rgb) = parse_hex_color(color.strip_prefix("rgb:").unwrap_or(color)) {
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
        "default" => fallback,
        _ => fallback,
    }
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

fn blend(color: u32, coverage: f32) -> u32 {
    let coverage = coverage.clamp(0.0, 1.0);
    let r = (((color >> 16) & 0xff) as f32 * coverage).round() as u32;
    let g = (((color >> 8) & 0xff) as f32 * coverage).round() as u32;
    let b = ((color & 0xff) as f32 * coverage).round() as u32;
    (r << 16) | (g << 8) | b
}

fn load_renderer() -> Result<Renderer> {
    let mut db = Database::new();
    db.load_system_fonts();
    let id = db
        .query(&Query {
            families: &[Family::Monospace],
            ..Query::default()
        })
        .context("could not find a monospace system font")?;

    let font = db
        .with_face_data(id, |data, _| FontArc::try_from_vec(data.to_vec()))
        .context("could not load font bytes")??;

    let font_size = PxScale::from(40.0);
    let scaled = font.as_scaled(font_size);
    let cell_width = scaled.h_advance(font.glyph_id('M')).ceil().max(1.0) as usize;
    let ascent = scaled.ascent().ceil();
    let descent = scaled.descent().abs().ceil() as usize;
    let line_gap = scaled.line_gap().ceil().max(0.0) as usize;
    let cell_height = ascent.max(1.0) as usize + descent + line_gap;

    Ok(Renderer {
        font,
        font_size,
        cell_width,
        cell_height: cell_height.max(16),
        ascent,
    })
}
