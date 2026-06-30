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

    // Initialize the fallible session state BEFORE starting the TUI. If config
    // load or agent init fails, we surface a normal error on the standard
    // terminal and exit non-zero — rather than spawning a full-screen UI we'd
    // then block on (join) while the real error scrolled past under the
    // alternate screen.
    let config = rt
        .block_on(Config::load_or_init())
        .map_err(|e| anyhow!("config load failed: {e}"))?;
    let mut agent = Agent::from_config(&config)
        .map_err(|e| anyhow!("agent init failed ({e}); run `openhuman login` first"))?;

    let (tx_input, rx_input) = std::sync::mpsc::channel::<String>();
    let (tx_resp, rx_resp) = std::sync::mpsc::channel::<String>();

    let tui_thread = std::thread::spawn(move || super::tui::run_tui(tx_input, rx_resp));

    rt.block_on(async {
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

    // Propagate TUI init/render/input failures so a broken session exits
    // non-zero instead of looking like a clean exit.
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
