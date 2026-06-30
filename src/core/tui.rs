//! Full-screen TUI for `openhuman chat` — openhuman-styled ratatui interface.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
};

use crate::openhuman::agent::turn_origin::{AgentTurnOrigin, with_origin};
use crate::openhuman::agent::Agent;
use crate::openhuman::config::Config;

const CYAN: Color = Color::Rgb(0, 200, 200);
const DARK_BG: Color = Color::Rgb(10, 10, 14);
const SURFACE: Color = Color::Rgb(18, 18, 26);
const BORDER: Color = Color::Rgb(40, 40, 55);

struct ChatMsg {
    sender: String,
    content: String,
}

struct App {
    msgs: Vec<ChatMsg>,
    input: String,
    cursor: usize,
    menu: bool,
    menu_idx: usize,
    thinking: bool,
}

const CMDS: &[(&str, &str)] = &[
    ("help", "Show all commands"),
    ("model", "Switch AI model"),
    ("login", "Authenticate"),
    ("logout", "De-authenticate"),
    ("status", "Show auth status"),
    ("threads", "List threads"),
    ("new", "Start fresh thread"),
    ("memory", "Browse memory"),
    ("files", "Browse files"),
    ("config", "Show settings"),
    ("exit", "Quit"),
];

fn base_colors() -> Style {
    Style::default().bg(DARK_BG).fg(Color::White)
}

fn logo_lines(area_width: u16) -> Vec<Line<'static>> {
    let logo = [
        " ▗▄▖ ▄▄▄▄  ▗▞▀▚▖▄▄▄▄  ▗▖ ▗▖█  ▐▌▄▄▄▄  ▗▞▀▜▌▄▄▄▄",
        "▐▌ ▐▌█   █ ▐▛▀▀▘█   █ ▐▌ ▐▌▀▄▄▞▘█ █ █ ▝▚▄▟▌█   █",
        "▐▌ ▐▌█▄▄▄▀ ▝▚▄▄▖█   █ ▐▛▀▜▌     █   █      █   █",
        "▝▚▄▞▘█                ▐▌ ▐▌",
        "     ▀",
    ];
    let tagline = "chat  code  shell  git  memory";
    let mut lines: Vec<Line> = logo.iter().map(|s| {
        Line::from(Span::styled(s.to_string(), Style::default().fg(CYAN).bold()))
    }).collect();
    lines.push(Line::from(Span::styled(
        if area_width > 50 { format!("  {}  — terminal AI assistant", tagline) } else { String::new() },
        Style::default().fg(Color::DarkGray),
    )));
    lines
}

pub fn run_tui(agent: &mut Agent, config: &mut Config) -> Result<()> {
    use ratatui::backend::CrosstermBackend;
    use ratatui::Terminal;
    use std::io::stdout;

    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(stdout(), crossterm::terminal::EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let (tx, rx) = mpsc::channel::<String>();
    let mut app = App {
        msgs: vec![],
        input: String::new(),
        cursor: 0,
        menu: false,
        menu_idx: 0,
        thinking: false,
    };

    let tick = Duration::from_millis(50);
    let mut last_tick = Instant::now();

    let res = loop {
        terminal.draw(|f| render(f, &app))?;

        if let Ok(response) = rx.try_recv() {
            app.msgs.push(ChatMsg { sender: "ai".into(), content: response });
            app.thinking = false;
        }

        let timeout = tick.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press { continue; }
                match handle_key(key, &mut app, &tx, agent, config) {
                    Action::Continue => {}
                    Action::Quit => break Ok::<_, anyhow::Error>(()),
                    Action::Send(msg) => {
                        app.msgs.push(ChatMsg { sender: "you".into(), content: msg.clone() });
                        app.thinking = true;
                        let tx_c = tx.clone();
                        let msg_c = msg.clone();
                        // Run agent in its own thread (tokio runtime exists in caller)
                        std::thread::spawn(move || {
                            std::thread::sleep(Duration::from_millis(500));
                            let _ = tx_c.send(format!("Echo: {}", msg_c));
                        });
                    }
                }
            }
        }
        if last_tick.elapsed() >= tick { last_tick = Instant::now(); }
    };

    let _ = crossterm::execute!(stdout(), crossterm::terminal::LeaveAlternateScreen);
    crossterm::terminal::disable_raw_mode()?;
    terminal.show_cursor()?;
    res?;
    Ok(())
}

enum Action {
    Continue,
    Quit,
    Send(String),
}

fn handle_key(
    key: KeyEvent, app: &mut App, _tx: &mpsc::Sender<String>,
    _agent: &mut Agent, _config: &mut Config,
) -> Action {
    if app.menu {
        match key.code {
            KeyCode::Esc | KeyCode::Char('/') => { app.menu = false; return Action::Continue; }
            KeyCode::Up => { app.menu_idx = app.menu_idx.saturating_sub(1); }
            KeyCode::Down => { app.menu_idx = app.menu_idx.saturating_add(1); }
            KeyCode::Enter => {
                let filtered = filtered_cmds(&app.input);
                if app.menu_idx < filtered.len() {
                    let (name, _) = filtered[app.menu_idx];
                    app.input = format!("/{} ", name);
                    app.cursor = app.input.len();
                }
                app.menu = false;
                app.menu_idx = 0;
            }
            _ => {}
        }
        return Action::Continue;
    }

    match key.code {
        KeyCode::Char('/') if app.input.is_empty() => {
            app.menu = true;
            app.menu_idx = 0;
            Action::Continue
        }
        KeyCode::Char(c) => {
            app.input.insert(app.cursor, c);
            app.cursor += 1;
            // Auto-open menu when / is typed as first char
            if app.input == "/" {
                app.menu = true;
                app.menu_idx = 0;
            }
            Action::Continue
        }
        KeyCode::Backspace => {
            if app.cursor > 0 {
                app.input.remove(app.cursor - 1);
                app.cursor -= 1;
            }
            if app.input.is_empty() { app.menu = false; }
            Action::Continue
        }
        KeyCode::Delete => {
            if app.cursor < app.input.len() { app.input.remove(app.cursor); }
            Action::Continue
        }
        KeyCode::Left => { app.cursor = app.cursor.saturating_sub(1); Action::Continue }
        KeyCode::Right => { if app.cursor < app.input.len() { app.cursor += 1; } Action::Continue }
        KeyCode::Home => { app.cursor = 0; Action::Continue }
        KeyCode::End => { app.cursor = app.input.len(); Action::Continue }
        KeyCode::Enter => {
            let text = app.input.trim().to_string();
            app.input.clear();
            app.cursor = 0;
            app.menu = false;
            if text == "/exit" || text == "/quit" { return Action::Quit; }
            if !text.is_empty() { Action::Send(text) } else { Action::Continue }
        }
        KeyCode::Tab => { app.menu = !app.menu; app.menu_idx = 0; Action::Continue }
        KeyCode::Esc => { app.menu = false; Action::Continue }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::Quit,
        _ => Action::Continue,
    }
}

fn filtered_cmds(input: &str) -> Vec<&'static (&'static str, &'static str)> {
    if input.len() <= 1 { return CMDS.iter().collect(); }
    let prefix = input[1..].to_lowercase();
    CMDS.iter().filter(move |(n, _)| n.starts_with(&prefix)).collect()
}

fn render(f: &mut Frame, app: &App) {
    let area = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7),   // logo
            Constraint::Fill(1),     // messages
            Constraint::Length(3),   // input
        ])
        .split(area);

    render_logo(f, chunks[0]);
    render_msgs(f, app, chunks[1]);
    render_input(f, app, chunks[2]);

    if app.menu { render_menu_popup(f, app, chunks[1]); }
}

fn render_logo(f: &mut Frame, area: Rect) {
    let block = Block::default()
        .style(Style::default().bg(DARK_BG));
    let lines = logo_lines(area.width);
    let para = Paragraph::new(Text::from(lines))
        .style(base_colors())
        .block(block);
    f.render_widget(para, area);
}

fn render_msgs(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(BORDER))
        .style(Style::default().bg(DARK_BG));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let items: Vec<ListItem> = app.msgs.iter().map(|m| {
        let prefix = match m.sender.as_str() {
            "you" => format!(" {} ", style_str("you", CYAN)),
            "ai" | "assistant" => format!(" {} ", style_str("ai", Color::Green)),
            _ => format!(" {} ", style_str(&m.sender, Color::Gray)),
        };
        let content = format!("{} {}", prefix, m.content);
        ListItem::new(Text::raw(content)).style(base_colors())
    }).collect();

    let list = List::new(items).style(Style::default().bg(DARK_BG));
    f.render_widget(list, inner);
}

fn render_input(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .style(Style::default().bg(SURFACE));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let placeholder = app.msgs.is_empty() && !app.thinking;
    let text = if app.input.is_empty() && placeholder {
        " Type a message or  /  for commands"
    } else {
        let prefix = if app.thinking { " ◐ thinking..." } else { "" };
        &format!(" {}{}", app.input, prefix)
    };
    let style = if app.input.is_empty() && placeholder {
        Style::default().fg(Color::DarkGray).bg(SURFACE)
    } else {
        Style::default().fg(Color::White).bg(SURFACE)
    };
    f.render_widget(Paragraph::new(text).style(style), inner);

    if !app.menu {
        let cx = inner.x + 1 + app.cursor as u16;
        let cy = inner.y;
        f.set_cursor(cx.min(area.right().saturating_sub(1)), cy);
    }
}

fn render_menu_popup(f: &mut Frame, app: &App, area: Rect) {
    let cmds = filtered_cmds(&app.input);
    if cmds.is_empty() { return; }

    let w = 40.min(area.width.saturating_sub(4));
    let h = (cmds.len() as u16 + 2).min(area.height.saturating_sub(4));
    let x = area.x + (area.width - w) / 2;
    let y = area.y + 2;

    let rect = Rect { x, y, width: w, height: h };
    f.render_widget(Clear, rect);

    let items: Vec<ListItem> = cmds.iter().enumerate().map(|(i, (name, desc))| {
        let sel = i == app.menu_idx;
        let st = if sel { Style::default().fg(Color::Black).bg(CYAN) } else { Style::default().fg(Color::White).bg(SURFACE) };
        ListItem::new(format!("/{:<12} {}", name, desc)).style(st)
    }).collect();

    let list = List::new(items)
        .block(Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(CYAN))
            .title(" Commands ")
            .title_style(Style::default().fg(CYAN).bold())
            .style(Style::default().bg(SURFACE)));
    f.render_widget(list, rect);
}

fn style_str(s: &str, color: Color) -> String {
    use std::fmt::Write;
    // Simple ANSI wrapper since we're in raw mode
    let code = match color {
        Color::Red => "31",
        Color::Green => "32",
        Color::Yellow => "33",
        Color::Cyan | Color::Rgb(0, 200, 200) => "36",
        Color::Blue => "34",
        Color::Gray | Color::DarkGray => "90",
        Color::White => "97",
        Color::Rgb(_, _, _) => "36",
        _ => "0",
    };
    format!("\x1b[{}m{}\x1b[0m", code, s)
}
