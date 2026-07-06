//! `openhuman chat` — TUI-based interactive chat session.

use std::sync::mpsc;

use anyhow::{anyhow, Result};
use console::style;

use crate::openhuman::agent::turn_origin::{AgentTurnOrigin, with_origin};
use crate::openhuman::agent::Agent;
use crate::openhuman::config::rpc::load_and_apply_model_settings;
use crate::openhuman::config::ops::ModelSettingsPatch;
use crate::openhuman::config::Config;
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
    let mut agent = Agent::from_config(&config)
        .map_err(|e| anyhow!("agent init failed ({e}); run `openhuman login` first"))?;

    let (tx_input, rx_input) = mpsc::channel::<String>();
    let (tx_cmd, rx_cmd) = mpsc::channel::<AgentCmd>();
    let (tx_resp, rx_resp) = mpsc::channel::<String>();

    let tui_thread = std::thread::spawn(move || super::tui::run_tui(tx_input, tx_cmd, rx_resp));

    rt.block_on(async {
        loop {
            // Check both channels
            if let Ok(msg) = rx_input.try_recv() {
                if msg == "/exit" || msg == "/quit" { break; }
                match with_origin(AgentTurnOrigin::Cli, agent.run_single(&msg)).await {
                    Ok(response) => { let _ = tx_resp.send(response); }
                    Err(e) => { let _ = tx_resp.send(format!("Error: {}", e)); }
                }
            }
            if let Ok(cmd) = rx_cmd.try_recv() {
                match cmd {
                    AgentCmd::SwitchModel(name) => {
                        match load_and_apply_model_settings(ModelSettingsPatch {
                            default_model: Some(name.clone()), ..Default::default()
                        }).await {
                            Ok(_) => {
                                config.default_model = Some(name.clone());
                                match Agent::from_config(&config) {
                                    Ok(a) => {
                                        agent = a;
                                        let _ = tx_resp.send(format!("Switched to model: {}", name));
                                    }
                                    Err(e) => {
                                        let _ = tx_resp.send(format!("Model switch failed: {}", e));
                                    }
                                }
                            }
                            Err(e) => {
                                let _ = tx_resp.send(format!("Model switch failed: {}", e));
                            }
                        }
                    }
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
