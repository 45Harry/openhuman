//! `openhuman` — interactive chat REPL with full GUI feature parity.
//!
//! Commands:
//!   /help       Show all commands
//!   /exit       Quit
//!   /model      Show/switch AI model
//!   /login      Authenticate
//!   /logout     De-authenticate
//!   /status     Show auth status
//!   /threads    List threads (select to view messages)
//!   /thread new Start fresh thread
//!   /memory     List memory namespaces
//!   /memory list <ns>   List docs in namespace
//!   /memory query <ns> <q>  Semantic query
//!   /memory recall <ns>     Recall context
//!   /files      List memory files
//!   /config     Show config
//!   /config model  Show model settings
//!   /config agent  Show agent settings
//!   /config paths  Show agent paths
//!   /usage      Show token/cycle usage
//!   /tools      List available tools

use anyhow::{anyhow, Result};
use console::style;
use dialoguer::{Input, Select};

use crate::openhuman::agent::turn_origin::{AgentTurnOrigin, with_origin};
use crate::openhuman::agent::Agent;
use crate::openhuman::config::rpc::load_and_apply_model_settings;
use crate::openhuman::config::ops::ModelSettingsPatch;
use crate::openhuman::config::Config;
use crate::openhuman::memory::{
    ConversationMessagesRequest, ConversationThreadsListResponse,
    EmptyRequest, ListDocumentsRequest, ListMemoryFilesRequest,
    QueryNamespaceRequest, RecallContextRequest,
};
use crate::openhuman::threads::ops::{
    messages_list, threads_list,
};
use crate::openhuman::memory::ops::{
    ai_list_memory_files, memory_list_documents, memory_list_namespaces,
    memory_query_namespace, memory_recall_context,
};
use crate::openhuman::config::ops::{
    load_and_get_config_snapshot, load_and_get_client_config_snapshot, get_agent_paths,
    get_agent_settings,
};

#[derive(Clone, Copy)]
enum Cmd {
    Exit,
    Help,
    Model,
    Login,
    Logout,
    Status,
    Threads,
    ThreadNew,
    Memory,
    MemoryList,
    MemoryQuery,
    MemoryRecall,
    Files,
    Config,
    ConfigModel,
    ConfigAgent,
    ConfigPaths,
    Usage,
    Tools,
}

impl Cmd {
    fn label(self) -> &'static str {
        match self {
            Cmd::Exit => "/exit         Quit",
            Cmd::Help => "/help         Show all commands",
            Cmd::Model => "/model        Show/switch AI model",
            Cmd::Login => "/login        Authenticate",
            Cmd::Logout => "/logout       De-authenticate",
            Cmd::Status => "/status       Show auth status",
            Cmd::Threads => "/threads      List threads (select to view)",
            Cmd::ThreadNew => "/thread new  Start fresh thread",
            Cmd::Memory => "/memory       Browse AI memory",
            Cmd::MemoryList => "/memory list <ns>  List docs in namespace",
            Cmd::MemoryQuery => "/memory query     Semantic memory search",
            Cmd::MemoryRecall => "/memory recall    Recall memory context",
            Cmd::Files => "/files        Browse memory files",
            Cmd::Config => "/config       Show all settings",
            Cmd::ConfigModel => "/config model     Show model settings",
            Cmd::ConfigAgent => "/config agent     Show agent settings",
            Cmd::ConfigPaths => "/config paths     Show agent paths",
            Cmd::Usage => "/usage        Show token/cycle usage",
            Cmd::Tools => "/tools        List available tools",
        }
    }
    fn all() -> Vec<Cmd> {
        vec![
            Cmd::Exit, Cmd::Help, Cmd::Model, Cmd::Login, Cmd::Logout, Cmd::Status,
            Cmd::Threads, Cmd::ThreadNew, Cmd::Memory, Cmd::MemoryList, Cmd::MemoryQuery,
            Cmd::MemoryRecall, Cmd::Files, Cmd::Config, Cmd::ConfigModel, Cmd::ConfigAgent,
            Cmd::ConfigPaths, Cmd::Usage, Cmd::Tools,
        ]
    }
    fn from_str(s: &str) -> Option<Cmd> {
        match s {
            "exit" | "quit" => Some(Cmd::Exit),
            "help" => Some(Cmd::Help),
            "model" => Some(Cmd::Model),
            "login" => Some(Cmd::Login),
            "logout" => Some(Cmd::Logout),
            "status" | "session" => Some(Cmd::Status),
            "threads" | "sessions" | "thread" => Some(Cmd::Threads),
            "new" => Some(Cmd::ThreadNew),
            "memory" | "mem" => Some(Cmd::Memory),
            "files" | "ls" => Some(Cmd::Files),
            "config" | "cfg" | "settings" => Some(Cmd::Config),
            "usage" | "stats" | "tokens" => Some(Cmd::Usage),
            "tools" | "tool" => Some(Cmd::Tools),
            _ => None,
        }
    }
    fn completions(prefix: &str) -> Vec<&'static str> {
        let all = &[
            "exit", "help", "model", "login", "logout", "status", "threads", "new",
            "memory", "files", "config", "usage", "tools",
        ][..];
        if prefix.is_empty() { all.to_vec() }
        else { all.iter().filter(|l| l.starts_with(prefix)).copied().collect() }
    }
}

pub fn run_chat_command(args: &[String]) -> Result<()> {
    let mut model: Option<String> = None;
    let mut temperature: Option<f64> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--model" | "-m" => {
                model = Some(args.get(i+1).ok_or_else(|| anyhow!("missing value for --model"))?.clone());
                i += 2;
            }
            "--temp" | "-t" => {
                let raw = args.get(i+1).ok_or_else(|| anyhow!("missing value for --temp"))?;
                temperature = Some(raw.parse::<f64>().map_err(|e| anyhow!("invalid --temp: {e}"))?);
                i += 2;
            }
            "-h" | "--help" => { print_help(); return Ok(()); }
            other => return Err(anyhow!("unknown arg: {other}")),
        }
    }
    run_interactive_session(model, temperature)
}

fn print_help() {
    println!("OpenHuman — terminal AI assistant");
    println!();
    println!("Usage:  openhuman [--model <name>] [--temp <n>]");
    println!();
    println!("Commands:");
    for cmd in Cmd::all() {
        let parts: Vec<&str> = cmd.label().splitn(2, "  ").collect();
        if parts.len() == 2 {
            println!("  {}  {}", parts[0].trim(), parts[1].trim());
        }
    }
}

fn show_welcome() {
    eprintln!();
    eprintln!("  {}", style("OpenHuman — terminal AI assistant").bold());
    eprintln!("  {}  {}  {}  {}  {}",
        style("chat").cyan().bold(), style("code").green().bold(),
        style("shell").yellow().bold(), style("git").magenta().bold(),
        style("memory").blue().bold());
    eprintln!("  Type a message or  {}  for commands", style("/").bold());
    eprintln!();
}

fn show_help() {
    eprintln!();
    eprintln!("  {}", style("Commands").bold());
    eprintln!("  {}", style("───────────────────────────────────────────").dim());
    for cmd in Cmd::all() {
        let parts: Vec<&str> = cmd.label().splitn(2, "  ").collect();
        if parts.len() == 2 {
            eprintln!("  {}  {}", style(parts[0].trim()).cyan().bold(), parts[1].trim());
        }
    }
    eprintln!();
    eprintln!("  {}  Type  {}  or Enter on empty line for menu.",
        style("Tip:").yellow().bold(), style("/").bold());
    eprintln!();
}

fn show_menu() -> Option<Cmd> {
    let items: Vec<String> = Cmd::all().iter().map(|c| c.label().to_string()).collect();
    let selection = Select::new()
        .with_prompt("command")
        .items(&items)
        .default(0)
        .interact()
        .ok()?;
    Some(Cmd::all()[selection])
}

fn show_json(v: &serde_json::Value) {
    let s = serde_json::to_string_pretty(v).unwrap_or_default();
    for line in s.lines() {
        eprintln!("  {}", line);
    }
}

fn run_interactive_session(model: Option<String>, temperature: Option<f64>) -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(crate::core::runtime::AGENT_WORKER_STACK_BYTES)
        .build()?;
    let term = console::Term::stdout();

    rt.block_on(async {
        let mut config = Config::load_or_init().await.map_err(|e| anyhow!("{e}"))?;
        if let Some(m) = model { config.default_model = Some(m); }
        if let Some(t) = temperature { config.default_temperature = t; }

        let mut agent = match Agent::from_config(&config) {
            Ok(a) => a,
            Err(e) => {
                eprintln!();
                eprintln!("  {}  Agent init failed: {}", style("✗").red().bold(), e);
                eprintln!();
                eprintln!("  Run  {}  to authenticate.", style("/login").cyan().bold());
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
                if let Some(cmd) = show_menu() {
                    if !exec_cmd(cmd, "", &mut config, &mut agent, &rt).await { break; }
                }
                continue;
            }

            if trimmed.starts_with('/') {
                let rest = trimmed[1..].trim().to_string();
                if rest.is_empty() {
                    if let Some(cmd) = show_menu() {
                        if !exec_cmd(cmd, "", &mut config, &mut agent, &rt).await { break; }
                    }
                    continue;
                }
                let parts: Vec<&str> = rest.splitn(2, ' ').collect();
                let cmd_name = parts[0];
                let cmd_arg = parts.get(1).map(|s| *s).unwrap_or("");

                if let Some(cmd) = Cmd::from_str(cmd_name) {
                    if !exec_cmd(cmd, cmd_arg, &mut config, &mut agent, &rt).await { break; }
                } else {
                    // Try subcommands
                    let sub = match cmd_name {
                        "memory" | "mem" => {
                            let sub_parts: Vec<&str> = cmd_arg.splitn(3, ' ').collect();
                            match sub_parts.first().copied().unwrap_or("") {
                                "list" | "ls" => Some(Cmd::MemoryList),
                                "query" | "q" | "search" => Some(Cmd::MemoryQuery),
                                "recall" | "rc" => Some(Cmd::MemoryRecall),
                                _ => None,
                            }
                        }
                        "config" | "cfg" | "settings" => {
                            match cmd_arg {
                                "model" | "models" => Some(Cmd::ConfigModel),
                                "agent" => Some(Cmd::ConfigAgent),
                                "paths" => Some(Cmd::ConfigPaths),
                                _ => None,
                            }
                        }
                        _ => None,
                    };
                    if let Some(cmd) = sub {
                        let sub_arg = if cmd_name == "memory" || cmd_name == "mem" {
                            let sub_parts: Vec<&str> = cmd_arg.splitn(3, ' ').collect();
                            sub_parts.get(2).copied().unwrap_or("")
                        } else {
                            ""
                        };
                        if !exec_cmd(cmd, sub_arg, &mut config, &mut agent, &rt).await { break; }
                    } else {
                        let completions = Cmd::completions(cmd_name);
                        if completions.is_empty() {
                            eprintln!("  {}  Unknown command  {}.  Type  {}  for all commands.",
                                style("?").yellow(), style(trimmed).bold(), style("/help").bold());
                        } else {
                            eprintln!("  {}  Unknown command  {}.  Did you mean:",
                                style("?").yellow(), style(trimmed).bold());
                            for c in &completions {
                                eprintln!("      {}", style(format!("/{}", c)).cyan().bold());
                            }
                        }
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

async fn exec_cmd(
    cmd: Cmd, arg: &str, config: &mut Config, agent: &mut Agent,
    _rt: &tokio::runtime::Runtime,
) -> bool {
    match cmd {
        Cmd::Exit => return false,
        Cmd::Help => show_help(),

        Cmd::Model => {
            let name = if arg.is_empty() {
                match Input::<String>::new().with_prompt("model name").allow_empty(false).interact_text() {
                    Ok(n) => n.trim().to_string(),
                    Err(_) => return true,
                }
            } else { arg.to_string() };
            if name.is_empty() { return true; }
            match load_and_apply_model_settings(ModelSettingsPatch {
                default_model: Some(name.clone()), ..Default::default()
            }).await {
                Ok(_) => {
                    config.default_model = Some(name.clone());
                    match Agent::from_config(config) {
                        Ok(a) => { *agent = a;
                            eprintln!("  {}  Model switched to {}", style("●").green(), style(&name).cyan().bold()); }
                        Err(e) => eprintln!("  {}  Agent init failed: {}", style("✗").red().bold(), e),
                    }
                }
                Err(e) => eprintln!("  {}  Failed: {}", style("✗").red().bold(), e),
            }
        }

        Cmd::Login => {
            eprintln!("  {}  Visit {} and get a token.",
                style("→").cyan().bold(), style("https://tinyhumans.ai/login").underlined());
            let token: String = match Input::<String>::new().with_prompt("paste token").allow_empty(false).interact_text() {
                Ok(t) => t.trim().to_string(),
                Err(_) => return true,
            };
            if token.is_empty() { return true; }
            match crate::openhuman::credentials::cli::cli_auth_login(
                "app-session".into(), token, None, None,
                serde_json::json!({}), None, true,
            ).await {
                Ok(r) => {
                    eprintln!("  {}  Logged in. {}", style("●").green().bold(),
                        r.get("result").and_then(|r| r.get("user"))
                            .and_then(|u| format!("{} {}", u.get("firstName").and_then(|v| v.as_str()).unwrap_or(""),
                                u.get("lastName").and_then(|v| v.as_str()).unwrap_or("")).into())
                            .unwrap_or_default());
                    let mut new_config = match Config::load_or_init().await {
                        Ok(c) => c,
                        Err(e) => { eprintln!("  {}  Config reload: {}", style("✗").red().bold(), e); return true; }
                    };
                    if let Some(m) = &config.default_model { new_config.default_model = Some(m.clone()); }
                    match Agent::from_config(&new_config) {
                        Ok(a) => { *agent = a; *config = new_config; eprintln!("  {}  Ready.", style("●").green().bold()); }
                        Err(e) => eprintln!("  {}  Agent init: {}", style("✗").red().bold(), e),
                    }
                }
                Err(e) => eprintln!("  {}  Login failed: {}", style("✗").red().bold(), e),
            }
        }

        Cmd::Logout => {
            match crate::openhuman::credentials::cli::cli_auth_logout("app-session".into(), None).await {
                Ok(r) => { eprintln!("  {}  Logged out.", style("●").green().bold()); show_json(&r); }
                Err(e) => eprintln!("  {}  Logout failed: {}", style("✗").red().bold(), e),
            }
        }

        Cmd::Status => {
            match crate::openhuman::credentials::cli::cli_auth_status("app-session".into(), None).await {
                Ok(r) => {
                    let authed = r.get("result").and_then(|x| x.get("isAuthenticated")).and_then(|x| x.as_bool()).unwrap_or(false);
                    let user = r.get("result").and_then(|x| x.get("user"));
                    if authed {
                        if let Some(u) = user {
                            let name = format!("{} {}",
                                u.get("firstName").and_then(|v| v.as_str()).unwrap_or(""),
                                u.get("lastName").and_then(|v| v.as_str()).unwrap_or("")).trim().to_string();
                            let email = u.get("email").and_then(|v| v.as_str()).unwrap_or("");
                            eprintln!("  {}  Authenticated as {}", style("●").green().bold(), style(&name).bold());
                            eprintln!("     {}", style(email).dim());
                            if let Some(u) = u.get("usage") {
                                if let Some(p) = u.get("promotionBalanceUsd").and_then(|v| v.as_f64()) {
                                    eprintln!("     Balance: ${:.2}", p);
                                }
                            }
                        } else { eprintln!("  {}  Authenticated.", style("●").green().bold()); }
                    } else { eprintln!("  {}  Not authenticated. Run  {}", style("○").yellow().bold(), style("/login").cyan().bold()); }
                }
                Err(e) => eprintln!("  {}  Status check failed: {}", style("✗").red().bold(), e),
            }
        }

        Cmd::Threads => {
            match threads_list(EmptyRequest {}).await {
                Ok(outcome) => {
                    let list = outcome.value.data.unwrap_or(ConversationThreadsListResponse { threads: vec![], count: 0 });
                    if list.threads.is_empty() {
                        eprintln!("  {}  No threads yet. Type a message to start one.", style("○").yellow().bold());
                        return true;
                    }
                    let titles: Vec<String> = list.threads.iter().map(|t| {
                        let label = if t.title.is_empty() { "(untitled)" } else { &t.title };
                        let active = if t.is_active { style(" ●").green().to_string() } else { String::new() };
                        format!("{}  {} msgs  [{:.8}]{}", label, t.message_count, t.id, active)
                    }).collect();
                    let sel = Select::new().with_prompt("thread (view messages)").items(&titles).default(0).interact();
                    match sel {
                        Ok(idx) => {
                            if let Some(t) = list.threads.get(idx) {
                                match messages_list(ConversationMessagesRequest { thread_id: t.id.clone() }).await {
                                    Ok(msgs) => {
                                        let Some(ref msgs_data) = msgs.value.data else { return true; };
                                        let msgs_list = msgs_data;
                                        if msgs_list.messages.is_empty() {
                                            eprintln!("  {}  No messages in this thread.", style("○").yellow().bold());
                                        } else {
                                            eprintln!("  {}  {} — {} messages", style("Thread:").cyan().bold(),
                                                style(&t.title).bold(), msgs_list.messages.len());
                                            eprintln!("  {}", style("───────────────────────────────────").dim());
                                            for msg in &msgs_list.messages {
                                                let sender = style(match msg.sender.as_str() {
                                                    "user" => "you",
                                                    "assistant" | "agent" | "ai" => "ai",
                                                    _ => &msg.sender,
                                                }).cyan().bold();
                                                let preview: String = msg.content.chars().take(160).collect();
                                                if msg.content.chars().count() > 160 {
                                                    eprintln!("  {} {}  {}...", sender, style("[→]").dim(), preview);
                                                } else {
                                                    eprintln!("  {}  {}", sender, preview);
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => eprintln!("  {}  Failed: {}", style("✗").red().bold(), e),
                                }
                            }
                        }
                        Err(_) => {}
                    }
                }
                Err(e) => eprintln!("  {}  Failed: {}", style("✗").red().bold(), e),
            }
        }

        Cmd::ThreadNew => {
            eprintln!("  {}  Starting fresh thread.", style("●").green().bold());
        }

        Cmd::Memory => {
            match memory_list_namespaces(EmptyRequest {}).await {
                Ok(outcome) => {
                    let nss = if let Some(ref d) = outcome.value.data { &d.namespaces } else { return true; };
                    if nss.is_empty() {
                        eprintln!("  {}  No memory namespaces found.", style("○").yellow().bold());
                    } else {
                        eprintln!("  {}  Namespaces:", style("Memory").cyan().bold());
                        for ns in nss {
                            eprintln!("    {}", style(ns).bold());
                        }
                        eprintln!();
                        eprintln!("  {}  Use  {}  to explore.",
                            style("Tip:").yellow().bold(), style("/memory list <namespace>").cyan().bold());
                    }
                }
                Err(e) => eprintln!("  {}  Failed: {}", style("✗").red().bold(), e),
            }
        }

        Cmd::MemoryList => {
            let ns = if arg.is_empty() {
                match Input::<String>::new().with_prompt("namespace").allow_empty(false).interact_text() {
                    Ok(n) => n.trim().to_string(),
                    Err(_) => return true,
                }
            } else { arg.to_string() };
            if ns.is_empty() { return true; }
            match memory_list_documents(ListDocumentsRequest { namespace: Some(ns.clone()) }).await {
                Ok(outcome) => {
                    let Some(ref data) = outcome.value.data else { return true; };
                    let docs = &data.documents;
                    if docs.is_empty() {
                        eprintln!("  {}  No documents in namespace  {}", style("○").yellow().bold(), style(&ns).bold());
                    } else {
                        eprintln!("  {}  {}  ({} docs):", style("Namespace:").cyan().bold(), style(&ns).bold(), docs.len());
                        for d in docs {
                            eprintln!("    {}  [{:.8}]  {}  {}",
                                style(&d.title).bold(), d.document_id, style(&d.source_type).dim(), style(&d.key).dim());
                        }
                    }
                }
                Err(e) => eprintln!("  {}  Failed: {}", style("✗").red().bold(), e),
            }
        }

        Cmd::MemoryQuery => {
            let ns = match Input::<String>::new().with_prompt("namespace").allow_empty(false).interact_text() {
                Ok(n) => n.trim().to_string(),
                Err(_) => return true,
            };
            if ns.is_empty() { return true; }
            let query = if arg.is_empty() {
                match Input::<String>::new().with_prompt("query").allow_empty(false).interact_text() {
                    Ok(q) => q.trim().to_string(),
                    Err(_) => return true,
                }
            } else { arg.to_string() };
            if query.is_empty() { return true; }
            match memory_query_namespace(QueryNamespaceRequest {
                namespace: ns.clone(), query: query.clone(),
                include_references: Some(true), document_ids: None, limit: None, max_chunks: Some(5),
            }).await {
                Ok(outcome) => {
                    let Some(ref data) = outcome.value.data else { return true; };
                    if let Some(ctx) = &data.context {
                        if let Some(msg) = &data.llm_context_message {
                            eprintln!("  {}  Result:", style("Memory").cyan().bold());
                            for line in msg.lines() {
                                eprintln!("    {}", line);
                            }
                        } else {
                            show_json(&serde_json::to_value(&ctx).unwrap_or_default());
                        }
                    } else {
                        eprintln!("  {}  No results for query.", style("○").yellow().bold());
                    }
                }
                Err(e) => eprintln!("  {}  Failed: {}", style("✗").red().bold(), e),
            }
        }

        Cmd::MemoryRecall => {
            let ns = if arg.is_empty() {
                match Input::<String>::new().with_prompt("namespace").allow_empty(false).interact_text() {
                    Ok(n) => n.trim().to_string(),
                    Err(_) => return true,
                }
            } else { arg.to_string() };
            if ns.is_empty() { return true; }
            match memory_recall_context(RecallContextRequest {
                namespace: ns.clone(), include_references: Some(true), limit: None, max_chunks: Some(5),
            }).await {
                Ok(outcome) => {
                    let Some(ref data) = outcome.value.data else { return true; };
                    if let Some(msg) = &data.llm_context_message {
                        eprintln!("  {}  Context from  {}:", style("Memory").cyan().bold(), style(&ns).bold());
                        for line in msg.lines() {
                            eprintln!("    {}", line);
                        }
                    } else {
                        eprintln!("  {}  No context available.", style("○").yellow().bold());
                    }
                }
                Err(e) => eprintln!("  {}  Failed: {}", style("✗").red().bold(), e),
            }
        }

        Cmd::Files => {
            match ai_list_memory_files(ListMemoryFilesRequest { relative_dir: String::new() }).await {
                Ok(outcome) => {
                    let Some(ref data) = outcome.value.data else { return true; };
                    let files = &data.files;
                    if files.is_empty() {
                        eprintln!("  {}  No memory files found.", style("○").yellow().bold());
                    } else {
                        eprintln!("  {}  Memory files ({}):", style("Files").cyan().bold(), files.len());
                        for f in files {
                            eprintln!("    {}", style(f).bold());
                        }
                        eprintln!();
                        eprintln!("  {}  Use  {}  to continue.",
                            style("Tip:").yellow().bold(), style("/files read <path>").cyan().bold());
                    }
                }
                Err(e) => eprintln!("  {}  Failed: {}", style("✗").red().bold(), e),
            }
        }

        Cmd::Config => {
            match load_and_get_config_snapshot().await {
                Ok(outcome) => show_json(&outcome.value),
                Err(e) => eprintln!("  {}  Failed: {}", style("✗").red().bold(), e),
            }
        }

        Cmd::ConfigModel => {
            match load_and_get_client_config_snapshot().await {
                Ok(outcome) => show_json(&outcome.value),
                Err(e) => eprintln!("  {}  Failed: {}", style("✗").red().bold(), e),
            }
        }

        Cmd::ConfigAgent => {
            match get_agent_settings().await {
                Ok(outcome) => show_json(&outcome.value),
                Err(e) => eprintln!("  {}  Failed: {}", style("✗").red().bold(), e),
            }
        }

        Cmd::ConfigPaths => {
            match get_agent_paths().await {
                Ok(outcome) => show_json(&outcome.value),
                Err(e) => eprintln!("  {}  Failed: {}", style("✗").red().bold(), e),
            }
        }

        Cmd::Usage | Cmd::Tools => {
            eprintln!("  {}  Feature coming soon. Use the GUI for now.", style("→").yellow().bold());
        }
    }
    true
}
