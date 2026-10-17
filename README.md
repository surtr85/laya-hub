# ⚡ Laya-Hub

> **High-performance intelligent supervisor and dynamic router for Model Context Protocol (MCP) servers and Agent Skills, accelerated by Laya System-1 on AMD Vulkan.**

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust: 2021](https://img.shields.io/badge/Rust-2021-orange.svg)](https://www.rust-lang.org/)
[![NixOS: Flake](https://img.shields.io/badge/NixOS-Declarative-5277C3.svg)](https://nixos.org)

---

## 🎯 The Problem

Modern AI agents and coding harnesses (Pi, Claude Desktop, Cursor, DeepSeek Harness) suffer from severe **Context Window Bloat** and **Tool Thrashing**:
- Exposing 100+ tools simultaneously consumes tens of thousands of tokens per prompt turn.
- Models hallucinate parameters, pick inappropriate subtools, and fail under tool overload.
- Each client harness requires independent configuration, disparate process lifecycles, and duplicate secrets.

## 💡 The Solution

**Laya-Hub** acts as a unified, high-speed MCP gateway and process supervisor:
1. **Single Source of Truth**: All MCP servers (local stdio, remote HTTP/SSE) and Skills (Markdown workflows) are declared in one central YAML file.
2. **Dynamic Capability Routing (<50ms)**: Instead of flooding the agent with 100+ tools, Laya-Hub uses **Laya System-1** (non-autoregressive ModernBERT on Vulkan) to score incoming requests and return only the relevant tools and full skill markdown instructions.
3. **On-Demand Lifecycle (Zero Memory Waste)**: Subprocesses are spawned only when called and automatically killed after an idle timeout.
4. **Universal Client Support**: Exposes a compact, standard 4-tool MCP interface (`hub_route`, `hub_call`, `hub_tools`, `hub_status`) compatible with any MCP client.

```text
[ Pi / DSH / Cursor / Claude ]
              |
              | (JSON-RPC over stdio)
              v
     +-----------------+
     |    laya-hub     | <--- ~/.config/laya-hub/config.yaml
     +--------+--------+
              |
      +-------+-------------------------+
      |                                 |
      v                                 v
[ Laya System-1 ]            [ On-Demand Supervisors ]
(Vulkan Port 8089)           |-- github-mcp-server
Score & Rank (<50ms)         |-- miku-researcher
                             |-- composio (HTTP/SSE)
                             +-- agent-memory
```

---

## 🚀 Features

- **⚡ Blazing Fast Routing**: Sub-50ms non-autoregressive routing via Laya System-1.
- **🛡️ Process Lifecycle Supervisor**: Manages child processes, pipes, stdio line buffering, and automated idle reaping.
- **📚 Native Skill Injection**: Markdown-based skill instructions are automatically injected into the agent context on demand.
- **🔒 Declarative Nix & Flakes Ready**: Zero impurity, reproducible packaging, and environment interpolation (`env:VAR`, `file:/path`).
- **🌐 HTTP & Stdio Transports**: Seamlessly route to local CLI tools or remote cloud streams (like Composio).

---

## 📦 Quick Start

### 1. Build & Run
```bash
cargo build --release
./target/release/laya-hub --config ~/.config/laya-hub/config.yaml
```

### 2. Test Routing CLI Mode
```bash
./target/release/laya-hub --test-route "Investigate why the GitHub PR check failed"
```

### 3. Declarative Configuration (`config.yaml`)

```yaml
version: "1.0"

laya:
  url: "http://127.0.0.1:8089/v1/systemone"
  threshold: 0.35
  max_active_tools: 3
  idle_timeout_secs: 180

mcp_servers:
  github:
    transport: stdio
    command: "github-mcp-server"
    description: "GitHub PRs, issues, commits, branches, repo code search"
    tags: [git, github, repo, pr, issue]

  composio:
    transport: http
    url: "https://connect.composio.dev/mcp"
    headers:
      x-consumer-api-key: "env:COMPOSIO_API_KEY"
    description: "Remote cloud integrations: Google Drive, Telegram, Google Calendar, Linear"
    tags: [cloud, gdrive, telegram, calendar, external]

skills:
  systematic_debugging:
    path: "~/.pi/agent/skills/systematic-debugging/SKILL.md"
    description: "Systematic root-cause tracing and hypothesis verification before code changes"
    tags: [debug, fix, issue, bug, error]
```

---

## 🛠️ MCP Tools Exposed

| Tool | Parameters | Description |
|---|---|---|
| `hub_route` | `task: string` | Scores task via Laya System-1, returns top matched MCP tools and injects full Markdown skill guides. |
| `hub_call` | `server: string, tool: string, arguments: object` | Spawns target server on demand, executes tool call, and returns result. |
| `hub_tools` | `server: string, tool?: string, refresh?: boolean` | Introspects available tools and JSON schemas for any server with thread-safe caching. |
| `hub_status` | none | Reports registered servers, active running child processes, and Laya status. |

---

## 🧠 Agent Skill Integration

LayaHub provides an agent skill definition compliant with standard agent harnesses (`pi`, `skills.sh`, Claude Code, Antigravity, DSH) located at [`skills/laya-hub/SKILL.md`](skills/laya-hub/SKILL.md).

To install or link the skill into your local agent skills directory:
```bash
mkdir -p ~/.pi/agent/skills/laya-hub
cp skills/laya-hub/SKILL.md ~/.pi/agent/skills/laya-hub/
```

---

## 📜 License

MIT © 2026 Sajjad (surtr85)
