//! `openhuman chat` — TUI-based interactive chat session.

use std::sync::mpsc;

use anyhow::{anyhow, Result};
use console::style;

use crate::core::tui::AgentCmd;
use crate::core::types::{approval_gate_boot_decision, HostKind};
use crate::openhuman::agent::turn_origin::{with_origin, AgentTurnOrigin};
use crate::openhuman::agent::Agent;
use crate::openhuman::approval::gate::{record_boot_state, ApprovalGateBootState};
use crate::openhuman::approval::{
    parse_approval_reply, ApprovalChatContext, ApprovalDecision, ApprovalGate,
    APPROVAL_CHAT_CONTEXT,
};
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
    install_cli_approval_gate(&config);
    let mut agent = match build_cli_agent(&config) {
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
    let cli_thread_id = format!("cli-chat-{}", uuid::Uuid::new_v4());
    let cli_client_id = "openhuman-cli".to_string();
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
                            let approval_ctx = ApprovalChatContext {
                                thread_id: cli_thread_id.clone(),
                                client_id: cli_client_id.clone(),
                            };
                            let origin = AgentTurnOrigin::WebChat {
                                thread_id: cli_thread_id.clone(),
                                client_id: cli_client_id.clone(),
                                request_id: Some(format!("cli-turn-{}", uuid::Uuid::new_v4())),
                            };
                            let turn = with_origin(
                                origin,
                                APPROVAL_CHAT_CONTEXT
                                    .scope(approval_ctx, active_agent.run_single(&msg)),
                            );
                            tokio::pin!(turn);
                            loop {
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
                                        break;
                                    }
                                    signal = wait_for_cli_approval_reply(
                                        &rx_input,
                                        &rx_quit,
                                        &tx_resp,
                                        &cli_thread_id,
                                    ) => {
                                        match signal {
                                            CliTurnSignal::ApprovalHandled => continue,
                                            CliTurnSignal::Quit => {
                                                log::debug!("[chat_cli] TUI quit signal received during agent turn; cancelling session");
                                                break;
                                            }
                                        }
                                    }
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

enum CliTurnSignal {
    ApprovalHandled,
    Quit,
}

async fn wait_for_cli_approval_reply(
    rx_input: &mpsc::Receiver<String>,
    rx_quit: &mpsc::Receiver<()>,
    tx_resp: &mpsc::Sender<String>,
    thread_id: &str,
) -> CliTurnSignal {
    let mut prompted_request_id: Option<String> = None;
    loop {
        if tui_requested_quit(rx_quit) {
            return CliTurnSignal::Quit;
        }

        let Some(gate) = ApprovalGate::try_global() else {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            continue;
        };
        let Some(request_id) = gate.pending_for_thread(thread_id) else {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            continue;
        };

        if prompted_request_id.as_deref() != Some(request_id.as_str()) {
            let _ = tx_resp.send(format_cli_approval_prompt(&gate, &request_id));
            prompted_request_id = Some(request_id.clone());
        }

        match rx_input.try_recv() {
            Ok(reply) => {
                if reply == "/exit" || reply == "/quit" {
                    return CliTurnSignal::Quit;
                }
                let Some(decision) = parse_approval_reply(&reply) else {
                    let _ = tx_resp.send(
                        "Approval pending. Reply yes/approve to allow once, or no/deny to block."
                            .to_string(),
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    continue;
                };
                match gate.decide(&request_id, decision) {
                    Ok(Some(row)) => {
                        log::debug!(
                            "[chat_cli] approval decision applied request_id={} tool={} decision={}",
                            row.request_id,
                            row.tool_name,
                            decision.as_str()
                        );
                        let _ =
                            tx_resp.send(format_cli_approval_decision(decision, &row.tool_name));
                    }
                    Ok(None) => {
                        log::debug!(
                            "[chat_cli] approval decision found no pending row request_id={request_id}"
                        );
                        let _ = tx_resp
                            .send("Approval request was already resolved or expired.".to_string());
                    }
                    Err(err) => {
                        log::debug!(
                            "[chat_cli] approval decision failed request_id={request_id}: {err}"
                        );
                        let _ = tx_resp.send(format!("Approval error: {err}"));
                    }
                }
                return CliTurnSignal::ApprovalHandled;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => return CliTurnSignal::Quit,
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
    match build_cli_agent(config) {
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
    let chat_provider = config
        .chat_provider
        .as_deref()
        .and_then(|current| rewrite_chat_provider_model(current, &name));
    match load_and_apply_model_settings(ModelSettingsPatch {
        default_model: Some(name.clone()),
        chat_provider: chat_provider.clone(),
        ..Default::default()
    })
    .await
    {
        Ok(_) => {
            config.default_model = Some(name.clone());
            if let Some(route) = chat_provider {
                config.chat_provider = Some(route);
            }
            match build_cli_agent(config) {
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
        Ok(reloaded) => match build_cli_agent(&reloaded) {
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
        },
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

fn install_cli_approval_gate(config: &Config) {
    let env_override_requested = std::env::var("OPENHUMAN_APPROVAL_GATE")
        .map(|v| {
            let trimmed = v.trim();
            trimmed == "0" || trimmed.eq_ignore_ascii_case("false")
        })
        .unwrap_or(false);
    let decision = approval_gate_boot_decision(HostKind::Cli, env_override_requested);
    record_boot_state(ApprovalGateBootState {
        installed: decision.install_gate,
        disabled_by_env: decision.gate_disabled_by_override,
        override_ignored: decision.override_ignored,
        host: HostKind::Cli.tag(),
    });

    if decision.install_gate {
        let session_id = format!("session-{}", uuid::Uuid::new_v4());
        let _ = ApprovalGate::init_global(config.clone(), session_id.clone());
        log::info!(
            "[chat_cli] approval gate installed for interactive terminal chat session_id={session_id}"
        );
    } else {
        log::warn!(
            "[chat_cli] approval gate disabled by OPENHUMAN_APPROVAL_GATE for interactive terminal chat"
        );
    }
}

fn build_cli_agent(config: &Config) -> Result<Agent> {
    let mut agent = Agent::from_config(config)?;
    let session_name = fresh_cli_session_name();
    agent.set_agent_definition_name(session_name.clone());
    agent.set_event_context(session_name, "cli");
    Ok(agent)
}

fn fresh_cli_session_name() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("orchestrator_cli_{}_{}", std::process::id(), nanos)
}

fn rewrite_chat_provider_model(current: &str, model: &str) -> Option<String> {
    let current = current.trim();
    let model = model.trim();
    if current.is_empty() || model.is_empty() {
        return None;
    }
    let (provider, _) = current.split_once(':')?;
    let provider = provider.trim();
    if provider.is_empty() {
        return None;
    }
    Some(format!("{provider}:{model}"))
}

fn format_cli_approval_prompt(gate: &ApprovalGate, request_id: &str) -> String {
    match gate.list_pending() {
        Ok(rows) => rows
            .into_iter()
            .find(|row| row.request_id == request_id)
            .map(|row| {
                format!(
                    "Approval required for tool '{}': {}\nReply yes/approve to allow once, or no/deny to block.",
                    row.tool_name, row.action_summary
                )
            })
            .unwrap_or_else(|| {
                "Approval required. Reply yes/approve to allow once, or no/deny to block."
                    .to_string()
            }),
        Err(err) => {
            log::debug!(
                "[chat_cli] failed to load pending approval details request_id={request_id}: {err}"
            );
            "Approval required. Reply yes/approve to allow once, or no/deny to block.".to_string()
        }
    }
}

fn format_cli_approval_decision(decision: ApprovalDecision, tool_name: &str) -> String {
    if decision.is_approve() {
        format!("Approved '{tool_name}' once.")
    } else {
        format!("Denied '{tool_name}'.")
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

#[cfg(test)]
mod tests {
    use super::{format_cli_approval_decision, rewrite_chat_provider_model};
    use crate::openhuman::approval::ApprovalDecision;

    #[test]
    fn rewrite_chat_provider_model_preserves_provider_prefix() {
        assert_eq!(
            rewrite_chat_provider_model("openai:gpt-4o", "gpt-5"),
            Some("openai:gpt-5".to_string())
        );
        assert_eq!(
            rewrite_chat_provider_model(" ollama:llama3 ", " qwen2.5 "),
            Some("ollama:qwen2.5".to_string())
        );
    }

    #[test]
    fn rewrite_chat_provider_model_leaves_unqualified_routes_alone() {
        assert_eq!(rewrite_chat_provider_model("cloud", "gpt-5"), None);
        assert_eq!(rewrite_chat_provider_model("", "gpt-5"), None);
        assert_eq!(rewrite_chat_provider_model("openai:gpt-4o", " "), None);
    }

    #[test]
    fn cli_approval_decision_message_reflects_decision() {
        assert_eq!(
            format_cli_approval_decision(ApprovalDecision::ApproveOnce, "shell"),
            "Approved 'shell' once."
        );
        assert_eq!(
            format_cli_approval_decision(ApprovalDecision::Deny, "shell"),
            "Denied 'shell'."
        );
    }
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
