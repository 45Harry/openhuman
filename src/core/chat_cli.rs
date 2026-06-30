//! `openhuman` — interactive chat REPL (Claude Code-style).
//!
//! Features:
//!   - Interactive chat with full agent (code, shell, git, MCP, memory)
//!   - `/` command menu (type `/` or Enter on empty line)
//!   - `/model <name>` — switch model mid-session
//!   - `/login` — authenticate with OpenHuman backend
//!   - `/session` — show current auth status
//!   - `/exit` — quit
//!
//! Usage:
//!   openhuman [--model <name>] [--temp <n>]

use anyhow::{anyhow, Result};
use console::style;
use dialoguer::{Input, Select};

use crate::openhuman::agent::turn_origin::{AgentTurnOrigin, with_origin};
use crate::openhuman::agent::Agent;
use crate::openhuman::config::rpc::load_and_apply_model_settings;
use crate::openhuman::config::ops::ModelSettingsPatch;
use crate::openhuman::config::Config;
use crate::openhuman::credentials::cli::cli_auth_status;

#[derive(Clone, Copy)]
enum Cmd {
    Exit,
    Help,
    Model,
    Login,
    Session,
}

impl Cmd {
    fn label(self) -> &'static str {
        match self {
            Cmd::Exit => "/exit     End session",
            Cmd::Help => "/help     Show commands",
            Cmd::Model => "/model    Switch AI model",
            Cmd::Login => "/login    Authenticate with OpenHuman",
            Cmd::Session => "/session  Show session & auth info",
        }
    }
    fn all() -> &'static [Cmd] {
        &[Cmd::Exit, Cmd::Help, Cmd::Model, Cmd::Login, Cmd::Session]
    }
}

pub fn run_chat_command(args: &[String]) -> Result<()> {
    let mut model: Option<String> = None;
    let mut temperature: Option<f64> = None;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--model" | "-m" => {
                model = Some(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow!("missing value for --model"))?
                        .clone(),
                );
                i += 2;
            }
            "--temp" | "-t" => {
                let raw = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("missing value for --temp"))?;
                temperature = Some(
                    raw.parse::<f64>()
                        .map_err(|e| anyhow!("invalid --temp: {e}"))?,
                );
                i += 2;
            }
            "-h" | "--help" => {
                print_help();
                return Ok(());
            }
            other => return Err(anyhow!("unknown arg: {other}")),
        }
    }

    run_interactive_session(model, temperature)
}

fn show_welcome() {
    eprintln!();
    eprintln!("  {}", style("OpenHuman — terminal AI assistant").bold());
    eprintln!("  {}  {}  {}", style("chat").cyan().bold(), style("code").green().bold(), style("shell").yellow().bold());
    eprintln!("  Type a message or  {}  for commands", style("/").bold());
    eprintln!();
}

fn show_menu() -> Option<Cmd> {
    let items: Vec<_> = Cmd::all().iter().map(|c| c.label()).collect();
    let selection = Select::new()
        .with_prompt("command")
        .items(&items)
        .default(0)
        .interact()
        .ok()?;
    Some(Cmd::all()[selection])
}

fn show_help() {
    eprintln!();
    eprintln!("  {}", style("Commands").bold());
    eprintln!("  {}", style("───────────────────────").dim());
    for cmd in Cmd::all() {
        let parts: Vec<&str> = cmd.label().splitn(2, "  ").collect();
        if parts.len() == 2 {
            eprintln!("  {}  {}", style(parts[0].trim()).cyan().bold(), parts[1].trim());
        }
    }
    eprintln!("  {}  {}", style("/model <name>").cyan().bold(), "Switch model (e.g. gpt-4o, claude-sonnet)");
    eprintln!();
    eprintln!("  {}  Just type anything to chat.", style("Tip:").yellow().bold());
    eprintln!("  {}  Type  {}  or press Enter on empty line for menu.", style("Tip:").yellow().bold(), style("/").bold());
    eprintln!();
}

fn print_help() {
    println!("OpenHuman — terminal AI assistant");
    println!();
    println!("Usage:");
    println!("  openhuman [--model <name>] [--temp <n>]");
    println!();
    println!("Options:");
    println!("  --model, -m <name>  Override the default model");
    println!("  --temp, -t <n>      Sampling temperature");
    println!();
    println!("Interactive commands (type / or Enter on empty line for menu):");
    for cmd in Cmd::all() {
        let parts: Vec<&str> = cmd.label().splitn(2, "  ").collect();
        if parts.len() == 2 {
            println!("  {}  {}", parts[0].trim(), parts[1].trim());
        }
    }
}

fn run_interactive_session(
    model: Option<String>,
    temperature: Option<f64>,
) -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(crate::core::runtime::AGENT_WORKER_STACK_BYTES)
        .build()?;

    let term = console::Term::stdout();

    rt.block_on(async {
        let mut config = Config::load_or_init().await.map_err(|e| anyhow!("{e}"))?;

        if let Some(m) = model {
            config.default_model = Some(m);
        }
        if let Some(t) = temperature {
            config.default_temperature = t;
        }

        let mut agent = match Agent::from_config(&config) {
            Ok(a) => a,
            Err(e) => {
                eprintln!();
                eprintln!("  {}  Agent init failed: {}", style("✗").red().bold(), e);
                eprintln!();
                eprintln!("  Make sure you're logged in or have a local model configured.");
                return Err(anyhow!("{e}"));
            }
        };

        show_welcome();

        loop {
            let input: String = match Input::new()
                .with_prompt(style("you").cyan().bold().to_string())
                .allow_empty(true)
                .interact_text()
            {
                Ok(text) => text,
                Err(_) => break,
            };

            let trimmed = input.trim().to_string();

            if trimmed == "/" || trimmed.is_empty() {
                match show_menu() {
                    Some(cmd) => {
                        match cmd {
                            Cmd::Exit => break,
                            Cmd::Help => show_help(),
                            Cmd::Model => {
                                let name: String = match Input::<String>::new()
                                    .with_prompt("model name")
                                    .allow_empty(false)
                                    .interact_text()
                                {
                                    Ok(n) => n.trim().to_string(),
                                    Err(_) => continue,
                                };
                                if name.is_empty() { continue; }
                                match load_and_apply_model_settings(ModelSettingsPatch {
                                    default_model: Some(name.clone()),
                                    ..Default::default()
                                }).await {
                                    Ok(_) => {
                                        config.default_model = Some(name.clone());
                                        match Agent::from_config(&config) {
                                            Ok(a) => {
                                                agent = a;
                                                eprintln!("  {}  Model switched to {}", style("●").green(), style(&name).cyan().bold());
                                            }
                                            Err(e) => eprintln!("  {}  Failed to init agent with new model: {}", style("✗").red().bold(), e),
                                        }
                                    }
                                    Err(e) => eprintln!("  {}  Failed to save model: {}", style("✗").red().bold(), e),
                                }
                            }
                            Cmd::Login => {
                                eprintln!("  {}  To log in, visit:", style("→").cyan().bold());
                                eprintln!("      {}", style("https://tinyhumans.ai/login").underlined());
                                eprintln!("  {}  Then use the CLI token from your account settings.", style("→").cyan().bold());
                                let token: String = match Input::<String>::new()
                                    .with_prompt("paste token")
                                    .allow_empty(false)
                                    .interact_text()
                                {
                                    Ok(t) => t.trim().to_string(),
                                    Err(_) => continue,
                                };
                                if token.is_empty() { continue; }
                                match crate::openhuman::credentials::cli::cli_auth_login(
                                    "app-session".into(), token, None, None,
                                    serde_json::json!({}), None, true,
                                ).await {
                                    Ok(_) => eprintln!("  {}  Logged in.", style("●").green().bold()),
                                    Err(e) => eprintln!("  {}  Login failed: {}", style("✗").red().bold(), e),
                                }
                            }
                            Cmd::Session => {
                                match cli_auth_status("app-session".into(), None).await {
                                    Ok(v) => {
                                        let s = serde_json::to_string_pretty(&v).unwrap_or_default();
                                        for line in s.lines() {
                                            eprintln!("  {}", line);
                                        }
                                    }
                                    Err(e) => eprintln!("  {}  Not logged in: {}", style("○").yellow().bold(), e),
                                }
                            }
                        }
                    }
                    None => break,
                }
                continue;
            }

            if trimmed.starts_with('/') {
                let parts: Vec<&str> = trimmed.splitn(2, ' ').collect();
                let cmd_name = parts[0];
                let cmd_arg = parts.get(1).map(|s| s.trim()).unwrap_or("");

                match cmd_name {
                    "/exit" | "/quit" => break,
                    "/help" => show_help(),
                    "/model" => {
                        if cmd_arg.is_empty() {
                            eprintln!("  {}  Usage: /model <name>", style("?").yellow());
                            continue;
                        }
                        match load_and_apply_model_settings(ModelSettingsPatch {
                            default_model: Some(cmd_arg.to_string()),
                            ..Default::default()
                        }).await {
                            Ok(_) => {
                                config.default_model = Some(cmd_arg.to_string());
                                match Agent::from_config(&config) {
                                    Ok(a) => {
                                        agent = a;
                                        eprintln!("  {}  Model switched to {}", style("●").green(), style(cmd_arg).cyan().bold());
                                    }
                                    Err(e) => eprintln!("  {}  Failed to init agent: {}", style("✗").red().bold(), e),
                                }
                            }
                            Err(e) => eprintln!("  {}  Failed to save model: {}", style("✗").red().bold(), e),
                        }
                    }
                    "/login" => {
                        eprintln!("  {}  Visit {} and get a token from settings.", style("→").cyan().bold(), style("https://tinyhumans.ai/login").underlined());
                        let token: String = match Input::<String>::new()
                            .with_prompt("paste token")
                            .allow_empty(false)
                            .interact_text()
                        {
                            Ok(t) => t.trim().to_string(),
                            Err(_) => continue,
                        };
                        if token.is_empty() { continue; }
                        match crate::openhuman::credentials::cli::cli_auth_login(
                            "app-session".into(), token, None, None,
                            serde_json::json!({}), None, true,
                        ).await {
                            Ok(_) => eprintln!("  {}  Logged in.", style("●").green().bold()),
                            Err(e) => eprintln!("  {}  Login failed: {}", style("✗").red().bold(), e),
                        }
                    }
                    "/session" => {
                        match cli_auth_status("app-session".into(), None).await {
                            Ok(v) => {
                                let s = serde_json::to_string_pretty(&v).unwrap_or_default();
                                for line in s.lines() {
                                    eprintln!("  {}", line);
                                }
                            }
                            Err(e) => eprintln!("  {}  Not logged in: {}", style("○").yellow().bold(), e),
                        }
                    }
                    _ => {
                        eprintln!("  {}  Unknown command. Type  {}  for menu.", style("?").yellow(), style("/").bold());
                    }
                }
                continue;
            }

            eprintln!("  {}  Thinking...", style("◐").dim());
            match with_origin(AgentTurnOrigin::Cli, agent.run_single(&trimmed)).await {
                Ok(response) => {
                    let _ = term.clear_last_lines(1);
                    if !response.trim().is_empty() {
                        term.write_line("")?;
                        term.write_str(&format!("  {}  ", style("▸").cyan().bold()))?;
                        term.write_line(response.trim())?;
                    }
                    term.write_line("")?;
                }
                Err(e) => {
                    let _ = term.clear_last_lines(1);
                    eprintln!("  {}  Error: {}", style("✗").red().bold(), e);
                }
            }
        }

        eprintln!();
        eprintln!("  {}  Session ended.", style("●").dim());
        eprintln!();

        Ok::<_, anyhow::Error>(())
    })?;

    Ok(())
}
