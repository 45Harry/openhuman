# OpenHuman CLI

> Your Personal AI super intelligence — now in your terminal.

The OpenHuman CLI brings the full power of OpenHuman to your command line. Chat with your AI, edit files, run commands, manage git, search the web, and more — all without leaving the terminal.

```bash
npm install -g openhuman
openhuman
```

## Install

### npm (macOS, Linux, Windows)

```bash
npm install -g openhuman
```

This downloads the pre-built Rust binary for your platform. Requires Node.js 18+.

### Homebrew (macOS)

```bash
brew tap tinyhumansai/core
brew install openhuman
```

### From source

```bash
git clone https://github.com/tinyhumansai/openhuman.git
cd openhuman
cargo build --bin openhuman-core
./target/debug/openhuman-core chat
```

## Usage

### Interactive chat

```bash
openhuman chat
```

Starts an interactive REPL. The agent remembers context across turns.

```
 ╔══════════════════════════════════════════╗
 ║    OpenHuman Interactive Chat           ║
 ║  /help for commands  /exit to quit      ║
 ╚══════════════════════════════════════════╝

you> write a python script to rename all .jpg files in the current dir to yyyy-mm-dd format

assistant> I'll create a script that renames .jpg files using their EXIF date or file modification time...

you> now run it on my ~/Photos directory
```

### One-shot query

```bash
openhuman call --method openhuman.agent_chat --params '{"message": "explain git rebase in one sentence"}'
```

### Server mode

```bash
openhuman run
```

Starts the HTTP/JSON-RPC server (default: `127.0.0.1:7788`). The `chat` command auto-starts this if needed.

### Available commands

| Command | Description |
|---------|-------------|
| `openhuman chat` | Interactive chat REPL |
| `openhuman run` | Start the HTTP/JSON-RPC server |
| `openhuman call --method <name> --params <json>` | Call an RPC method |
| `openhuman agent list` | List agent definitions |
| `openhuman agent dump-prompt --agent <id>` | Inspect agent prompts |
| `openhuman memory` | Memory inspection & ingestion |
| `openhuman mcp` | MCP stdio server |
| `openhuman --help` | Full command list |

### Options

```bash
openhuman chat --model gpt-4o    # use a specific model
openhuman chat --temp 0.7        # set temperature
openhuman chat -v                # verbose logging
```

## Interactive commands (inside chat)

| Command | Description |
|---------|-------------|
| `/exit`, `/quit` | End session |
| `/help` | Show commands |

## Features

- **Full agent toolset** — filesystem read/write, bash, git, web search, web scraping
- **100+ OAuth integrations** — Gmail, Notion, GitHub, Slack, Calendar, Drive, Linear, Jira
- **5,000+ MCP servers** — plug into the Model Context Protocol ecosystem
- **90,000+ Skills** — installable skill catalog
- **Memory Tree** — persistent, local-first knowledge base (SQLite + Obsidian vault)
- **SuperContext** — agent has relevant context on turn 1 via automatic memory sweep
- **Smart token compression** — TokenJuice reduces LLM costs by up to 80%
- **Model routing** — automatic selection of the best model per workload
- **Goals & Todos** — long-term goals, thread goals, kanban task boards

## Configuration

Config is auto-initialized on first run at `~/.openhuman/config.toml`.

Override the core server address:
```bash
export OPENHUMAN_CORE_HOST=127.0.0.1
export OPENHUMAN_CORE_PORT=7788
```

Environment file: set `OPENHUMAN_DOTENV_PATH` to load a custom `.env`.

## Development

```bash
# build the CLI binary
cargo build --bin openhuman-core

# run the chat REPL
./target/debug/openhuman-core chat

# run tests
cargo test -p openhuman
```

## How it works

The CLI bundles the OpenHuman Rust core as a native binary. The `chat` subcommand loads the agent in-process with full access to all tools and memory — no separate server needed for interactive sessions.

For the Node.js wrapper (`openhuman chat` via npm), it auto-starts the core server as a child process and communicates over HTTP JSON-RPC, giving you the same agent with automatic process management.

## Comparison

| Feature | Claude Code | OpenHuman CLI |
|---------|-------------|---------------|
| Open source | Proprietary | GPL-3.0 |
| Local memory | Chat-scoped | Persistent Memory Tree + Obsidian vault |
| Integrations | Few connectors | 100+ OAuth, 5k+ MCP, 90k+ Skills |
| Model routing | Single model | Built-in routing |
| Token compression | None | TokenJuice (up to 80% savings) |
| Cost | Subscription + add-ons | One subscription, all models |
| Desktop app | ✅ | ✅ (same agent, GUI + CLI) |
| Auto-fetch context | ❌ | 20-min auto-sync |

## Links

- [GitHub](https://github.com/tinyhumansai/openhuman)
- [Documentation](https://tinyhumans.gitbook.io/openhuman/)
- [Discord](https://discord.tinyhumans.ai/)
- [Issues](https://github.com/tinyhumansai/openhuman/issues)
