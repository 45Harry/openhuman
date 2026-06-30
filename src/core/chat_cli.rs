//! `openhuman chat` — TUI-based interactive chat session.

use anyhow::{anyhow, Result};
use console::style;

use crate::openhuman::agent::turn_origin::{AgentTurnOrigin, with_origin};
use crate::openhuman::agent::Agent;
use crate::openhuman::config::Config;

pub fn run_chat_command(args: &[String]) -> Result<()> {
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => { print_help(); return Ok(()); }
            other => return Err(anyhow!("unknown arg: {other}")),
        }
        i += 1;
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

    let (tx_input, rx_input) = std::sync::mpsc::channel::<String>();
    let (tx_resp, rx_resp) = std::sync::mpsc::channel::<String>();

    let tui_thread = std::thread::spawn(move || {
        super::tui::run_tui(tx_input, rx_resp)
    });

    rt.block_on(async {
        let mut config = match Config::load_or_init().await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("  {}  Config load failed: {}", style("✗").red().bold(), e);
                return;
            }
        };

        let mut agent = match Agent::from_config(&config) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("  {}  Agent init failed. Run  {}", style("✗").red().bold(),
                    style("openhuman login").cyan().bold());
                eprintln!("     {}", e);
                return;
            }
        };

        loop {
            let msg = match rx_input.recv() {
                Ok(m) => m,
                Err(_) => break,
            };
            if msg == "/exit" || msg == "/quit" { break; }
            match with_origin(AgentTurnOrigin::Cli, agent.run_single(&msg)).await {
                Ok(response) => { let _ = tx_resp.send(response); }
                Err(e) => { let _ = tx_resp.send(format!("Error: {}", e)); }
            }
        }
    });

    let _ = tui_thread.join();
    eprintln!();
    eprintln!("  {}  Session ended.", style("●").dim());
    eprintln!();
    Ok(())
}
