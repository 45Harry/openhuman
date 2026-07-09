//! `openhuman chat` — TUI-based interactive chat session.

use std::sync::mpsc;

use anyhow::{anyhow, Result};
use console::style;

use crate::core::tui::AgentCmd;
use crate::openhuman::agent::turn_origin::{with_origin, AgentTurnOrigin};
use crate::openhuman::agent::Agent;
use crate::openhuman::config::ops::ModelSettingsPatch;
use crate::openhuman::config::rpc::load_and_apply_model_settings;
use crate::openhuman::config::Config;
use crate::openhuman::cost::{try_global, CostTracker};
use crate::openhuman::credentials::ops::clear_session;
use crate::openhuman::credentials::session_support::build_session_state;
use crate::openhuman::memory::ops::{ai_list_memory_files, memory_list_namespaces};
use crate::openhuman::memory::rpc_models::{EmptyRequest, ListMemoryFilesRequest};
use crate::openhuman::memory_conversations::list_threads;

pub fn run_chat_command(args: &[String]) -> Result<()> {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print_help();
        return Ok(());
    }
    run_interactive_session()
}

fn print_help() {
    println!("OpenHuman — terminal AI assistant");
    println!();
    println!("Usage:  openhuman chat");
    println!();
    println!("Starts a full-screen TUI chat session.");
    println!();
    println!("Controls:");
    println!("  /          Open command menu");
    println!("  Tab        Toggle menu");
    println!("  Enter      Send message / select command");
    println!("  Esc        Close menu");
    println!("  Ctrl+C     Quit");
    println!("  ↑ ↓        Navigate menu / scroll");
    println!("  ← → Home End  Cursor navigation");
}

fn run_interactive_session() -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(crate::core::runtime::AGENT_WORKER_STACK_BYTES)
        .build()?;

    let mut config = rt
        .block_on(Config::load_or_init())
        .map_err(|e| anyhow!("config load failed: {e}"))?;
    config.action_dir = std::env::current_dir().map_err(|e| anyhow!("failed to get cwd: {e}"))?;
    let mut agent = match Agent::from_config(&config) {
        Ok(agent) => Some(agent),
        Err(e) => {
            log::debug!("[chat_cli] agent init failed before TUI start: {e}");
            None
        }
    };

    let (tx_input, rx_input) = mpsc::channel::<String>();
    let (tx_cmd, rx_cmd) = mpsc::channel::<AgentCmd>();
    let (tx_quit, rx_quit) = mpsc::channel::<()>();
    let (tx_resp, rx_resp) = mpsc::channel::<String>();

    let initial_model = config
        .default_model
        .clone()
        .unwrap_or_else(|| "unknown".into());
    if agent.is_none() {
        let _ = tx_resp.send(agent_not_ready_message());
    }
    let tui_thread = std::thread::spawn(move || {
        crate::core::tui::run_tui(tx_input, tx_cmd, tx_quit, rx_resp, &initial_model)
    });

    rt.block_on(async {
        loop {
            if tui_requested_quit(&rx_quit) {
                log::debug!("[chat_cli] TUI quit signal received; ending session");
                break;
            }
            match rx_input.try_recv() {
                Ok(msg) => {
                    if msg == "/exit" || msg == "/quit" {
                        break;
                    }
                    match agent.as_mut() {
                        Some(active_agent) => {
                            let turn =
                                with_origin(AgentTurnOrigin::Cli, active_agent.run_single(&msg));
                            tokio::pin!(turn);
                            tokio::select! {
                                result = &mut turn => {
                                    match result {
                                        Ok(response) => {
                                            let _ = tx_resp.send(response);
                                        }
                                        Err(e) => {
                                            log::debug!("[chat_cli] agent turn failed: {e}");
                                            let _ = tx_resp.send(format!("Error: {e}"));
                                        }
                                    }
                                }
                                _ = wait_for_tui_quit(&rx_quit) => {
                                    log::debug!("[chat_cli] TUI quit signal received during agent turn; cancelling session");
                                    break;
                                }
                            }
                        }
                        None => {
                            let _ = tx_resp.send(agent_not_ready_message());
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    log::debug!("[chat_cli] TUI input channel disconnected; ending session");
                    break;
                }
            }
            match rx_cmd.try_recv() {
                Ok(cmd) => {
                    let response = handle_cmd(cmd, &mut config, &mut agent).await;
                    let _ = tx_resp.send(response);
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    log::debug!("[chat_cli] TUI command channel disconnected; ending session");
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    });

    match tui_thread.join() {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(anyhow!("TUI error: {e}")),
        Err(_) => return Err(anyhow!("TUI thread panicked")),
    }
    eprintln!();
    eprintln!("  {}  Session ended.", style("●").dim());
    eprintln!();
    Ok(())
}

fn tui_requested_quit(rx_quit: &mpsc::Receiver<()>) -> bool {
    match rx_quit.try_recv() {
        Ok(_) | Err(mpsc::TryRecvError::Disconnected) => true,
        Err(mpsc::TryRecvError::Empty) => false,
    }
}

async fn wait_for_tui_quit(rx_quit: &mpsc::Receiver<()>) {
    loop {
        if tui_requested_quit(rx_quit) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

async fn handle_cmd(cmd: AgentCmd, config: &mut Config, agent: &mut Option<Agent>) -> String {
    match cmd {
        AgentCmd::NewConversation => handle_new_conversation(config, agent),
        AgentCmd::SwitchModel(name) => handle_switch_model(config, agent, name).await,
        AgentCmd::Login => handle_login(config, agent).await,
        AgentCmd::Logout => handle_logout(config, agent).await,
        AgentCmd::Status => format_status(config),
        AgentCmd::ListThreads => handle_list_threads(config).await,
        AgentCmd::ListMemory => handle_list_memory().await,
        AgentCmd::ListFiles => handle_list_files().await,
        AgentCmd::ShowConfig => format_config(config),
        AgentCmd::ShowUsage => handle_usage(config).await,
        AgentCmd::ListTools => match agent {
            Some(active_agent) => list_agent_tools(active_agent),
            None => agent_not_ready_message(),
        },
    }
}

fn handle_new_conversation(config: &Config, agent: &mut Option<Agent>) -> String {
    match Agent::from_config(config) {
        Ok(rebuilt) => {
            *agent = Some(rebuilt);
            "Started a new conversation.".into()
        }
        Err(e) => {
            log::debug!("[chat_cli] agent rebuild for new conversation failed: {e}");
            *agent = None;
            format!("Started a new conversation, but the agent is not ready.\n\nLast error: {e}")
        }
    }
}

async fn handle_switch_model(
    config: &mut Config,
    agent: &mut Option<Agent>,
    name: String,
) -> String {
    match load_and_apply_model_settings(ModelSettingsPatch {
        default_model: Some(name.clone()),
        ..Default::default()
    })
    .await
    {
        Ok(_) => {
            config.default_model = Some(name.clone());
            match Agent::from_config(config) {
                Ok(a) => {
                    *agent = Some(a);
                    format!("Switched to model: {name}")
                }
                Err(e) => {
                    log::debug!("[chat_cli] agent rebuild after model switch failed: {e}");
                    *agent = None;
                    format!("Model switch failed: {e}")
                }
            }
        }
        Err(e) => format!("Model switch failed: {e}"),
    }
}

async fn handle_login(config: &mut Config, agent: &mut Option<Agent>) -> String {
    match Config::load_or_init().await {
        Ok(mut reloaded) => {
            reloaded.action_dir = config.action_dir.clone();
            match Agent::from_config(&reloaded) {
                Ok(rebuilt) => {
                    *config = reloaded;
                    *agent = Some(rebuilt);
                    "Login state refreshed. Agent is ready.".into()
                }
                Err(e) => {
                    log::debug!("[chat_cli] login refresh did not initialize agent: {e}");
                    *config = reloaded;
                    *agent = None;
                    format!("{}\n\nLast error: {e}", login_instructions())
                }
            }
        }
        Err(e) => {
            log::debug!("[chat_cli] config reload during login failed: {e}");
            format!("{}\n\nConfig reload failed: {e}", login_instructions())
        }
    }
}

async fn handle_logout(config: &mut Config, agent: &mut Option<Agent>) -> String {
    match clear_session(config).await {
        Ok(_) => {
            *agent = None;
            "Logged out. Session cleared.".into()
        }
        Err(e) => format!("Logout failed: {e}"),
    }
}

fn agent_not_ready_message() -> String {
    format!("Agent is not ready yet.\n\n{}", login_instructions())
}

fn login_instructions() -> &'static str {
    "To log in, get your session token from the OpenHuman app and run:\n  openhuman auth store_session --token <token>\n\nOr set provider credentials in the app, then use /login here to refresh the session."
}

fn format_status(config: &Config) -> String {
    let mut lines = Vec::new();
    lines.push("── Status ──".into());
    if let Some(model) = &config.default_model {
        lines.push(format!("Model: {model}"));
    }
    match build_session_state(config) {
        Ok(state) => {
            if state.is_authenticated {
                lines.push("Auth: logged in".into());
            } else {
                lines.push("Auth: not logged in".into());
            }
        }
        Err(_) => lines.push("Auth: unknown".into()),
    }
    lines.push(format!("Workspace: {}", config.workspace_dir.display()));
    lines.push(format!("Action dir: {}", config.action_dir.display()));
    lines.join("\n")
}

async fn handle_list_threads(config: &Config) -> String {
    let workspace = config.workspace_dir.clone();
    match tokio::task::spawn_blocking(move || list_threads(workspace)).await {
        Ok(Ok(threads)) => {
            if threads.is_empty() {
                return "No conversation threads found.".into();
            }
            let mut lines = Vec::new();
            lines.push(format!("── Threads ({}) ──", threads.len()));
            for t in &threads {
                let title = if t.title.is_empty() {
                    "(untitled)".to_string()
                } else {
                    t.title.clone()
                };
                lines.push(format!("  {:<38} {} msgs", title, t.message_count));
            }
            lines.join("\n")
        }
        Ok(Err(e)) => format!("Failed to list threads: {e}"),
        Err(e) => format!("Thread listing failed: {e}"),
    }
}

async fn handle_list_memory() -> String {
    match memory_list_namespaces(EmptyRequest {}).await {
        Ok(outcome) => {
            let envelope = outcome.value;
            if let Some(data) = envelope.data {
                if data.namespaces.is_empty() {
                    return "No memory namespaces found.".into();
                }
                let mut lines = Vec::new();
                lines.push(format!("── Memory namespaces ({}) ──", data.count));
                for ns in &data.namespaces {
                    lines.push(format!("  {ns}"));
                }
                lines.join("\n")
            } else {
                "Memory system not available.".into()
            }
        }
        Err(e) => format!("Memory listing failed: {e}"),
    }
}

async fn handle_list_files() -> String {
    match ai_list_memory_files(ListMemoryFilesRequest {
        relative_dir: ".".into(),
    })
    .await
    {
        Ok(outcome) => {
            let envelope = outcome.value;
            if let Some(data) = envelope.data {
                if data.files.is_empty() {
                    return "No memory files found.".into();
                }
                let mut lines = Vec::new();
                lines.push(format!("── Memory files ({}) ──", data.count));
                for f in &data.files {
                    lines.push(format!("  {f}"));
                }
                lines.join("\n")
            } else {
                "No memory files found.".into()
            }
        }
        Err(e) => format!("File listing failed: {e}"),
    }
}

fn format_config(config: &Config) -> String {
    let mut lines = Vec::new();
    lines.push("── Config ──".into());
    lines.push(format!("Workspace: {}", config.workspace_dir.display()));
    lines.push(format!("Action dir: {}", config.action_dir.display()));
    lines.push(format!(
        "Default model: {}",
        config.default_model.as_deref().unwrap_or("not set")
    ));
    lines.push(format!(
        "API URL: {}",
        config.api_url.as_deref().unwrap_or("default")
    ));
    lines.push(format!(
        "Inference URL: {}",
        config.inference_url.as_deref().unwrap_or("default")
    ));
    lines.push(format!("Temperature: {}", config.default_temperature));
    lines.push(format!(
        "Output language: {}",
        config.output_language.as_deref().unwrap_or("default")
    ));
    lines.push(format!("Schema version: {}", config.schema_version));
    lines.join("\n")
}

async fn handle_usage(config: &Config) -> String {
    let tracker = try_global().or_else(|| {
        CostTracker::new(config.cost.clone(), &config.workspace_dir)
            .ok()
            .map(std::sync::Arc::new)
    });
    match tracker {
        Some(t) => match t.get_daily_history(7) {
            Ok(entries) => {
                if entries.is_empty() {
                    return "No usage data recorded yet.".into();
                }
                let total_cost: f64 = entries.iter().map(|e| e.cost_usd).sum();
                let total_tokens: u64 = entries.iter().map(|e| e.total_tokens).sum();
                let mut lines = Vec::new();
                lines.push(format!(
                    "── Usage (last 7 days) ──  ${:.4}  {} tokens",
                    total_cost, total_tokens
                ));
                for entry in &entries {
                    lines.push(format!(
                        "  {}  ${:.4}  {}i/{}o tokens",
                        entry.date, entry.cost_usd, entry.input_tokens, entry.output_tokens
                    ));
                }
                lines.join("\n")
            }
            Err(e) => format!("Usage query failed: {e}"),
        },
        None => "Usage tracking not initialized.".into(),
    }
}

fn list_agent_tools(agent: &Agent) -> String {
    let tools = agent.tools();
    if tools.is_empty() {
        return "No tools available.".into();
    }
    let mut lines = Vec::new();
    lines.push(format!("── Tools ({}) ──", tools.len()));
    for tool in tools {
        lines.push(format!("  {:<20} {}", tool.name(), tool.description()));
    }
    lines.join("\n")
}
