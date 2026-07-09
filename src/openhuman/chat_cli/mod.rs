//! `openhuman chat` — TUI-based interactive chat session.

use std::sync::mpsc;

use anyhow::{anyhow, Result};
use console::style;

use crate::openhuman::agent::turn_origin::{AgentTurnOrigin, with_origin};
use crate::openhuman::agent::Agent;
use crate::openhuman::config::rpc::load_and_apply_model_settings;
use crate::openhuman::config::ops::ModelSettingsPatch;
use crate::openhuman::config::Config;
use crate::openhuman::credentials::ops::clear_session;
use crate::openhuman::credentials::session_support::build_session_state;
use crate::openhuman::cost::{try_global, CostTracker};
use crate::openhuman::memory::ops::{
    ai_list_memory_files, memory_list_namespaces,
};
use crate::openhuman::memory::rpc_models::{
    EmptyRequest, ListMemoryFilesRequest,
};
use crate::openhuman::memory_conversations::list_threads;
use crate::core::tui::AgentCmd;

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
    config.action_dir = std::env::current_dir()
        .map_err(|e| anyhow!("failed to get cwd: {e}"))?;
    let mut agent = Agent::from_config(&config)
        .map_err(|e| anyhow!("agent init failed ({e}); run `openhuman login` first"))?;

    let (tx_input, rx_input) = mpsc::channel::<String>();
    let (tx_cmd, rx_cmd) = mpsc::channel::<AgentCmd>();
    let (tx_resp, rx_resp) = mpsc::channel::<String>();

    let initial_model = config.default_model.clone().unwrap_or_else(|| "unknown".into());
    let tui_thread = std::thread::spawn(move || super::tui::run_tui(tx_input, tx_cmd, rx_resp, &initial_model));

    rt.block_on(async {
        loop {
            if let Ok(msg) = rx_input.try_recv() {
                if msg == "/exit" || msg == "/quit" {
                    break;
                }
                match with_origin(AgentTurnOrigin::Cli, agent.run_single(&msg)).await {
                    Ok(response) => {
                        let _ = tx_resp.send(response);
                    }
                    Err(e) => {
                        let _ = tx_resp.send(format!("Error: {e}"));
                    }
                }
            }
            if let Ok(cmd) = rx_cmd.try_recv() {
                let response = handle_cmd(cmd, &mut config, &mut agent).await;
                let _ = tx_resp.send(response);
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

async fn handle_cmd(cmd: AgentCmd, config: &mut Config, agent: &mut Agent) -> String {
    match cmd {
        AgentCmd::SwitchModel(name) => handle_switch_model(config, agent, name).await,
        AgentCmd::Login => handle_login().await,
        AgentCmd::Logout => handle_logout(config).await,
        AgentCmd::Status => format_status(config),
        AgentCmd::ListThreads => handle_list_threads(config).await,
        AgentCmd::ListMemory => handle_list_memory().await,
        AgentCmd::ListFiles => handle_list_files().await,
        AgentCmd::ShowConfig => format_config(config),
        AgentCmd::ShowUsage => handle_usage(config).await,
        AgentCmd::ListTools => list_agent_tools(agent),
    }
}

async fn handle_switch_model(config: &mut Config, agent: &mut Agent, name: String) -> String {
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
                    *agent = a;
                    format!("Switched to model: {name}")
                }
                Err(e) => format!("Model switch failed: {e}"),
            }
        }
        Err(e) => format!("Model switch failed: {e}"),
    }
}

async fn handle_login() -> String {
    "To log in, get your API token from the OpenHuman app and run:\n  openhuman login <token>\n\nOr set OPENHUMAN_API_KEY in your environment.".into()
}

async fn handle_logout(config: &mut Config) -> String {
    match clear_session(config).await {
        Ok(_) => "Logged out. Session cleared.".into(),
        Err(e) => format!("Logout failed: {e}"),
    }
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
    lines.push(format!(
        "Temperature: {}",
        config.default_temperature
    ));
    lines.push(format!(
        "Output language: {}",
        config.output_language.as_deref().unwrap_or("default")
    ));
    lines.push(format!("Schema version: {}", config.schema_version));
    lines.join("\n")
}

async fn handle_usage(config: &Config) -> String {
    let tracker = try_global().or_else(|| {
        CostTracker::new(config.cost.clone(), &config.workspace_dir).ok().map(std::sync::Arc::new)
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
