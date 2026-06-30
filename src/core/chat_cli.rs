//! `openhuman chat` — interactive chat REPL for the OpenHuman agent.
//!
//! Start an interactive conversation with the agent directly from the terminal.
//! Maintains conversation history across turns so the agent remembers context.
//!
//! Usage:
//!   openhuman chat [--model <model>] [--temp <temp>] [-v]
//!
//! Interactive commands:
//!   /exit    Quit
//!   /help    Show commands

use anyhow::{anyhow, Result};
use console::Term;
use dialoguer::Input;

use crate::openhuman::agent::turn_origin::{AgentTurnOrigin, with_origin};
use crate::openhuman::agent::Agent;
use crate::openhuman::config::Config;

pub fn run_chat_command(args: &[String]) -> Result<()> {
    let mut model: Option<String> = None;
    let mut temperature: Option<f64> = None;
    let mut verbose = false;
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
            "-v" | "--verbose" => {
                verbose = true;
                i += 1;
            }
            "-h" | "--help" => {
                print_help();
                return Ok(());
            }
            other => return Err(anyhow!("unknown chat arg: {other}")),
        }
    }

    run_interactive_session(model, temperature, verbose)
}

fn run_interactive_session(
    model: Option<String>,
    temperature: Option<f64>,
    verbose: bool,
) -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(crate::core::runtime::AGENT_WORKER_STACK_BYTES)
        .build()?;

    let term = Term::stdout();

    rt.block_on(async {
        let mut config = Config::load_or_init().await.map_err(|e| anyhow!("{e}"))?;

        if let Some(m) = model {
            config.default_model = Some(m);
        }
        if let Some(t) = temperature {
            config.default_temperature = t;
        }

        let mut agent = Agent::from_config(&config).map_err(|e| anyhow!("{e}"))?;

        eprintln!();
        eprintln!(" ╔══════════════════════════════════════════╗");
        eprintln!(" ║    OpenHuman Interactive Chat           ║");
        eprintln!(" ║  /help for commands  /exit to quit      ║");
        eprintln!(" ╚══════════════════════════════════════════╝");
        eprintln!();

        loop {
            let input: String = match Input::new()
                .with_prompt("you")
                .allow_empty(true)
                .interact_text()
            {
                Ok(text) => text,
                Err(_) => break,
            };

            let trimmed = input.trim().to_string();
            if trimmed.is_empty() {
                continue;
            }

            if trimmed.starts_with('/') {
                match trimmed.as_str() {
                    "/exit" | "/quit" => break,
                    "/help" => print_interactive_help(),
                    _ => eprintln!("Unknown: {trimmed}"),
                }
                continue;
            }

            match with_origin(
                AgentTurnOrigin::Cli,
                agent.run_single(&trimmed),
            )
            .await
            {
                Ok(response) => {
                    term.write_line("")?;
                    term.write_str("assistant> ")?;
                    term.write_line(&response)?;
                    term.write_line("")?;
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                }
            }
        }

        Ok::<_, anyhow::Error>(())
    })?;

    Ok(())
}

fn print_help() {
    println!("OpenHuman interactive chat CLI");
    println!();
    println!("Usage:");
    println!("  openhuman chat [--model <name>] [--temp <n>] [-v]");
    println!();
    println!("Options:");
    println!("  --model, -m <name>  Override the default model");
    println!("  --temp, -t <n>      Sampling temperature");
    println!("  -v, --verbose       Verbose logging");
    println!();
    print_interactive_help();
}

fn print_interactive_help() {
    println!("Commands:");
    println!("  /exit, /quit  Quit");
    println!("  /help         This help");
    println!();
    println!("Type anything else to chat with the agent.");
}
