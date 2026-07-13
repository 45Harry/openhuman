use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

const CYAN: Color = Color::Rgb(0, 200, 200);
const DARK_BG: Color = Color::Rgb(10, 10, 14);
const SURFACE: Color = Color::Rgb(18, 18, 26);
const BORDER: Color = Color::Rgb(40, 40, 55);

pub enum AgentCmd {
    NewConversation,
    SwitchModel(String),
    Login,
    Logout,
    Status,
    ListThreads,
    ListMemory,
    ListFiles,
    ShowConfig,
    ShowUsage,
    ListTools,
}

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
    model_popup: bool,
    model_idx: usize,
    models: Vec<String>,
    scroll_offset: usize,
    model_name: String,
}

const CMDS: &[(&str, &str)] = &[
    ("help", "Show available commands"),
    ("model", "Switch AI model"),
    ("new", "Start new conversation"),
    ("login", "Authenticate with API token"),
    ("logout", "Clear saved credentials"),
    ("status", "Show auth status & current config"),
    ("threads", "List conversation threads"),
    ("memory", "Browse AI memory"),
    ("files", "View attached files"),
    ("config", "Show current configuration"),
    ("usage", "Display token usage"),
    ("tools", "List available tools"),
    ("exit", "Quit openhuman"),
];

const MODELS: &[&str] = &[
    "gpt-4o",
    "gpt-4o-mini",
    "claude-sonnet-4",
    "claude-haiku",
    "gemini-2.5-pro",
];

pub fn run_tui(
    tx_input: mpsc::Sender<String>,
    tx_cmd: mpsc::Sender<AgentCmd>,
    tx_quit: mpsc::Sender<()>,
    tx_approval: mpsc::Sender<String>,
    rx_resp: mpsc::Receiver<String>,
    initial_model: &str,
) -> Result<()> {
    use ratatui::backend::CrosstermBackend;
    use ratatui::Terminal;
    use std::io::stdout;

    crossterm::terminal::enable_raw_mode()?;
    if let Err(e) = crossterm::execute!(stdout(), crossterm::terminal::EnterAlternateScreen) {
        let _ = crossterm::terminal::disable_raw_mode();
        return Err(e.into());
    }
    let mut terminal = match Terminal::new(CrosstermBackend::new(stdout())) {
        Ok(t) => t,
        Err(e) => {
            let _ = crossterm::execute!(stdout(), crossterm::terminal::LeaveAlternateScreen);
            let _ = crossterm::terminal::disable_raw_mode();
            return Err(e.into());
        }
    };

    let mut app = App {
        msgs: vec![],
        input: String::new(),
        cursor: 0,
        menu: false,
        menu_idx: 0,
        thinking: false,
        model_popup: false,
        model_idx: 0,
        models: MODELS.iter().map(|s| s.to_string()).collect(),
        scroll_offset: 0,
        model_name: initial_model.into(),
    };

    let tick = Duration::from_millis(50);
    let mut last_tick = Instant::now();

    let res: Result<()> = (|| loop {
        terminal.draw(|f| render(f, &app))?;

        if let Ok(response) = rx_resp.try_recv() {
            app.msgs.push(ChatMsg {
                sender: "ai".into(),
                content: response.clone(),
            });
            if let Some(name) = response.strip_prefix("Switched to model: ") {
                app.model_name = name.to_string();
            }
            app.scroll_offset = 0;
            app.thinking = false;
        }

        let timeout = tick.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match handle_key(key, &mut app, &tx_input, &tx_cmd, &tx_approval) {
                    Action::Continue => {}
                    Action::Quit => {
                        let _ = tx_quit.send(());
                        return Ok(());
                    }
                    Action::Send(msg) => {
                        app.msgs.push(ChatMsg {
                            sender: "you".into(),
                            content: msg.clone(),
                        });
                        app.scroll_offset = 0;
                        app.thinking = true;
                        let _ = tx_input.send(msg);
                    }
                }
            }
        }
        if last_tick.elapsed() >= tick {
            last_tick = Instant::now();
        }
    })();

    let _ = crossterm::execute!(stdout(), crossterm::terminal::LeaveAlternateScreen);
    let _ = crossterm::terminal::disable_raw_mode();
    let _ = terminal.show_cursor();
    res
}

enum Action {
    Continue,
    Quit,
    Send(String),
}

fn handle_key(
    key: KeyEvent,
    app: &mut App,
    _tx_input: &mpsc::Sender<String>,
    tx_cmd: &mpsc::Sender<AgentCmd>,
    tx_approval: &mpsc::Sender<String>,
) -> Action {
    if app.model_popup {
        match key.code {
            KeyCode::Esc => {
                app.model_popup = false;
            }
            KeyCode::Up => {
                app.model_idx = app.model_idx.saturating_sub(1);
            }
            KeyCode::Down => {
                app.model_idx = app.model_idx.saturating_add(1).min(app.models.len() - 1);
            }
            KeyCode::Enter => {
                if app.model_idx < app.models.len() {
                    let model = app.models[app.model_idx].clone();
                    let _ = tx_cmd.send(AgentCmd::SwitchModel(model));
                }
                app.model_popup = false;
                app.model_idx = 0;
            }
            _ => {}
        }
        return Action::Continue;
    }

    if app.menu {
        match key.code {
            KeyCode::Esc | KeyCode::Char('/') => {
                app.menu = false;
                return Action::Continue;
            }
            KeyCode::Up => {
                app.menu_idx = app.menu_idx.saturating_sub(1);
            }
            KeyCode::Down => {
                app.menu_idx = app.menu_idx.saturating_add(1);
            }
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
            KeyCode::Char(c) => {
                app.input.insert(app.cursor, c);
                app.cursor += c.len_utf8();
                app.menu_idx = 0;
                if app.input == "/" {
                    app.menu_idx = 0;
                }
            }
            KeyCode::Backspace => {
                if let Some(prev) = app.input[..app.cursor].chars().next_back() {
                    app.cursor -= prev.len_utf8();
                    app.input.remove(app.cursor);
                }
                app.menu_idx = 0;
                if app.input.is_empty() || app.input == "/" {
                    app.menu = false;
                }
            }
            _ => {}
        }
        return Action::Continue;
    }

    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::Quit,
        KeyCode::Char('/') if app.input.is_empty() => {
            app.menu = true;
            app.menu_idx = 0;
            Action::Continue
        }
        KeyCode::Up if app.input.is_empty() => {
            app.scroll_offset = app.scroll_offset.saturating_add(1);
            Action::Continue
        }
        KeyCode::Down if app.input.is_empty() => {
            app.scroll_offset = app.scroll_offset.saturating_sub(1);
            Action::Continue
        }
        KeyCode::PageUp => {
            app.scroll_offset = app.scroll_offset.saturating_add(10);
            Action::Continue
        }
        KeyCode::PageDown => {
            app.scroll_offset = app.scroll_offset.saturating_sub(10);
            Action::Continue
        }
        KeyCode::Char(c) => {
            app.input.insert(app.cursor, c);
            app.cursor += c.len_utf8();
            if app.input == "/" {
                app.menu = true;
                app.menu_idx = 0;
            }
            Action::Continue
        }
        KeyCode::Backspace => {
            if let Some(prev) = app.input[..app.cursor].chars().next_back() {
                app.cursor -= prev.len_utf8();
                app.input.remove(app.cursor);
            }
            if app.input.is_empty() {
                app.menu = false;
            }
            Action::Continue
        }
        KeyCode::Delete => {
            if app.cursor < app.input.len() {
                app.input.remove(app.cursor);
            }
            Action::Continue
        }
        KeyCode::Left => {
            if let Some(prev) = app.input[..app.cursor].chars().next_back() {
                app.cursor -= prev.len_utf8();
            }
            Action::Continue
        }
        KeyCode::Right => {
            if let Some(next) = app.input[app.cursor..].chars().next() {
                app.cursor += next.len_utf8();
            }
            Action::Continue
        }
        KeyCode::Home => {
            app.cursor = 0;
            Action::Continue
        }
        KeyCode::End => {
            app.cursor = app.input.len();
            Action::Continue
        }
        KeyCode::Enter => {
            let text = app.input.trim().to_string();
            app.input.clear();
            app.cursor = 0;
            app.menu = false;
            if text.is_empty() {
                return Action::Continue;
            }
            if text == "/exit" || text == "/quit" {
                return Action::Quit;
            }
            if text == "/help" {
                cmds_list(app);
                return Action::Continue;
            }
            if text == "/model" {
                app.model_popup = true;
                app.model_idx = 0;
                return Action::Continue;
            }
            if text == "/new" {
                if app.thinking {
                    app.msgs.push(ChatMsg {
                        sender: "system".into(),
                        content: "Cannot start a new conversation while a turn is in progress. Please wait.".into(),
                    });
                    return Action::Continue;
                }
                app.msgs.clear();
                app.scroll_offset = 0;
                let _ = tx_cmd.send(AgentCmd::NewConversation);
                return Action::Continue;
            }
            if is_approval_reply(&text) {
                let _ = tx_approval.send(text.clone());
            }
            if text.starts_with('/') {
                let cmd = text.trim_start_matches('/').trim_start().to_string();
                let sent = match cmd.as_str() {
                    "login" => {
                        let _ = tx_cmd.send(AgentCmd::Login);
                        true
                    }
                    "logout" => {
                        let _ = tx_cmd.send(AgentCmd::Logout);
                        true
                    }
                    "status" => {
                        let _ = tx_cmd.send(AgentCmd::Status);
                        true
                    }
                    "threads" => {
                        let _ = tx_cmd.send(AgentCmd::ListThreads);
                        true
                    }
                    "memory" => {
                        let _ = tx_cmd.send(AgentCmd::ListMemory);
                        true
                    }
                    "files" => {
                        let _ = tx_cmd.send(AgentCmd::ListFiles);
                        true
                    }
                    "config" => {
                        let _ = tx_cmd.send(AgentCmd::ShowConfig);
                        true
                    }
                    "usage" => {
                        let _ = tx_cmd.send(AgentCmd::ShowUsage);
                        true
                    }
                    "tools" => {
                        let _ = tx_cmd.send(AgentCmd::ListTools);
                        true
                    }
                    other => {
                        app.msgs.push(ChatMsg {
                            sender: "system".into(),
                            content: format!("Unknown command /{other}. Try /help."),
                        });
                        false
                    }
                };
                if sent {
                    app.thinking = true;
                }
                return Action::Continue;
            }
            Action::Send(text)
        }
        KeyCode::Tab => {
            app.menu = !app.menu;
            app.menu_idx = 0;
            Action::Continue
        }
        KeyCode::Esc => {
            app.menu = false;
            app.model_popup = false;
            Action::Continue
        }
        _ => Action::Continue,
    }
}

fn cmds_list(app: &mut App) {
    let mut lines = vec![];
    for (name, desc) in CMDS {
        lines.push(format!("/{:<12} {}", name, desc));
    }
    app.msgs.push(ChatMsg {
        sender: "system".into(),
        content: lines.join("\n"),
    });
}

fn filtered_cmds(input: &str) -> Vec<&'static (&'static str, &'static str)> {
    let prefix = input.strip_prefix('/').unwrap_or(input);
    if prefix.is_empty() {
        return CMDS.iter().collect();
    }
    let prefix = prefix.to_lowercase();
    CMDS.iter()
        .filter(move |(n, _)| n.starts_with(&prefix))
        .collect()
}

fn is_approval_reply(text: &str) -> bool {
    let t = text.trim().to_lowercase();
    matches!(t.as_str(), "yes" | "y" | "approve" | "allow" | "no" | "n" | "deny" | "reject")
}

fn render(f: &mut Frame, app: &App) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Fill(1),
            Constraint::Length(1),
            Constraint::Length(3),
        ])
        .split(area);

    render_logo(f, chunks[0]);
    render_msgs(f, app, chunks[1]);
    render_status(f, app, chunks[2]);
    render_input(f, app, chunks[3]);

    if app.menu {
        render_menu_popup(f, app, chunks[1]);
    }
    if app.model_popup {
        render_model_popup(f, app, chunks[1]);
    }
}

fn logo_lines() -> Vec<Line<'static>> {
    [
        " ▗▄▖ ▄▄▄▄  ▗▞▀▚▖▄▄▄▄  ▗▖ ▗▖█  ▐▌▄▄▄▄  ▗▞▀▜▌▄▄▄▄",
        "▐▌ ▐▌█   █ ▐▛▀▀▘█   █ ▐▌ ▐▌▀▄▄▞▘█ █ █ ▝▚▄▟▌█   █",
        "▐▌ ▐▌█▄▄▄▀ ▝▚▄▄▖█   █ ▐▛▀▜▌     █   █      █   █",
        "▝▚▄▞▘█                ▐▌ ▐▌",
        "     ▀",
    ]
    .iter()
    .map(|s| {
        Line::from(Span::styled(
            s.to_string(),
            Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
        ))
    })
    .collect()
}

fn render_logo(f: &mut Frame, area: Rect) {
    let para = Paragraph::new(Text::from(logo_lines()))
        .style(Style::default().bg(DARK_BG))
        .block(Block::default().style(Style::default().bg(DARK_BG)));
    f.render_widget(para, area);
}

fn render_msgs(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(BORDER))
        .style(Style::default().bg(DARK_BG));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let items = msgs_to_items(app);
    let visible_count = inner.height as usize;
    let total = items.len();
    if total == 0 {
        return;
    }

    let max_offset = total.saturating_sub(visible_count);
    let offset = app.scroll_offset.min(max_offset);
    let start = total.saturating_sub(visible_count + offset);

    let end = (start + visible_count).min(total);
    let visible: Vec<ListItem> = items[start..end].to_vec();
    f.render_widget(
        List::new(visible).style(Style::default().bg(DARK_BG)),
        inner,
    );
}

fn msgs_to_items(app: &App) -> Vec<ListItem<'static>> {
    let mut items = Vec::new();
    for m in &app.msgs {
        let tag = match m.sender.as_str() {
            "you" => ansi_str("you", CYAN),
            "ai" | "assistant" => ansi_str("ai", Color::Green),
            "system" => ansi_str("sys", Color::Gray),
            _ => ansi_str(&m.sender, Color::Gray),
        };
        let mut first = true;
        for line in m.content.split('\n') {
            let text = if first {
                format!(" {} {}", tag, line)
            } else {
                format!("   {}", line)
            };
            items.push(ListItem::new(Text::raw(text)).style(Style::default().bg(DARK_BG)));
            first = false;
        }
    }
    items
}

fn render_status(f: &mut Frame, app: &App, area: Rect) {
    let msg = format!(
        " {}  Model: {}  |  Ctrl+C to quit  |  /  for menu ",
        ansi_str("●", Color::Green),
        app.model_name
    );
    let para =
        Paragraph::new(Text::raw(msg)).style(Style::default().bg(SURFACE).fg(Color::DarkGray));
    f.render_widget(para, area);
}

fn render_input(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .style(Style::default().bg(SURFACE));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let (text, style) = if app.model_popup {
        (
            " Select a model...  (↑↓ navigate, Enter select, Esc cancel)".into(),
            Style::default().fg(Color::Yellow).bg(SURFACE),
        )
    } else if app.thinking {
        (
            format!(" {} ◐ thinking...", app.input),
            Style::default().fg(Color::Yellow).bg(SURFACE),
        )
    } else if app.input.is_empty() && app.msgs.is_empty() {
        (
            " Type a message or  /  for commands".into(),
            Style::default().fg(Color::DarkGray).bg(SURFACE),
        )
    } else {
        (
            format!(" {}", app.input),
            Style::default().fg(Color::White).bg(SURFACE),
        )
    };

    f.render_widget(Paragraph::new(text).style(style), inner);
    if !app.menu && !app.model_popup {
        let cx = inner.x + 1 + app.cursor as u16;
        f.set_cursor_position((cx.min(area.right().saturating_sub(1)), inner.y));
    }
}

fn render_menu_popup(f: &mut Frame, app: &App, area: Rect) {
    let cmds = filtered_cmds(&app.input);
    if cmds.is_empty() {
        return;
    }
    let w = 40.min(area.width.saturating_sub(4));
    let h = (cmds.len() as u16 + 2).min(area.height.saturating_sub(4));
    let x = area.x + (area.width - w) / 2;
    let y = area.y + 2;
    let rect = Rect {
        x,
        y,
        width: w,
        height: h,
    };
    f.render_widget(Clear, rect);

    let items: Vec<ListItem> = cmds
        .iter()
        .enumerate()
        .map(|(i, (name, desc))| {
            let sel = i == app.menu_idx;
            let st = if sel {
                Style::default().fg(Color::Black).bg(CYAN)
            } else {
                Style::default().fg(Color::White).bg(SURFACE)
            };
            ListItem::new(format!("/{:<12} {}", name, desc)).style(st)
        })
        .collect();
    f.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(CYAN))
                .title(" Commands ")
                .title_style(Style::default().fg(CYAN).add_modifier(Modifier::BOLD))
                .style(Style::default().bg(SURFACE)),
        ),
        rect,
    );
}

fn render_model_popup(f: &mut Frame, app: &App, area: Rect) {
    if app.models.is_empty() {
        return;
    }
    let w = 34.min(area.width.saturating_sub(4));
    let h = (app.models.len() as u16 + 2).min(area.height.saturating_sub(4));
    let x = area.x + (area.width - w) / 2;
    let y = area.y + 2;
    let rect = Rect {
        x,
        y,
        width: w,
        height: h,
    };
    f.render_widget(Clear, rect);

    let items: Vec<ListItem> = app
        .models
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let sel = i == app.model_idx;
            let st = if sel {
                Style::default().fg(Color::Black).bg(CYAN)
            } else {
                Style::default().fg(Color::White).bg(SURFACE)
            };
            ListItem::new(m.to_string()).style(st)
        })
        .collect();
    f.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green))
                .title(" Models ")
                .title_style(
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )
                .style(Style::default().bg(SURFACE)),
        ),
        rect,
    );
}

fn ansi_str(s: &str, color: Color) -> String {
    let code = match color {
        Color::Green => "32",
        Color::Yellow => "33",
        Color::Cyan => "36",
        Color::Gray | Color::DarkGray => "90",
        Color::White => "97",
        _ => "0",
    };
    format!("\x1b[{}m{}\x1b[0m", code, s)
}
