---
name: laya-hub
description: Master control plane and capability router for AI coding agents connecting to registered MCP servers and skills via Laya System-1 non-autoregressive routing and on-demand lifecycle supervision.
---

# LayaHub Universal Skill (`laya-hub`)

**LayaHub (`surtr85/laya-hub`)** is the zero-bloat, ultra-low-latency MCP supervisor and dynamic capability gateway written in high-performance asynchronous Rust (`tokio`).

Instead of polluting LLM context windows with 160+ static tool definitions, all harnesses (Pi, Antigravity, DSH) connect exclusively to `laya_hub` and interact via three high-density meta-tools.

---

## 🏛️ Core Meta-Tools

### 1. `hub_route(task: string, min_score?: number)`
Evaluates the task using the local Laya System-1 Vulkan model (`http://127.0.0.1:8089/v1/systemone`) in < 50ms:
- Predicts matching MCP servers and relevance probabilities.
- Matches appropriate procedural skills (Persian + English tags & semantic matching).
- Returns the complete procedural Markdown instructions of matching skills directly into the agent context.

### 2. `hub_call(server: string, tool: string, arguments: object)`
- Dynamically spawns the target MCP server child process on-demand if not already running.
- Injects environment variables and credentials securely (`file:`, `env:`).
- Performs stdio or HTTP/SSE JSON-RPC call handshaking transparently.
- Returns the execution result directly.

### 3. `hub_status()`
Inspects:
- Active running subprocesses.
- Registered server definitions.
- Registered skills.
- Laya engine configuration and idle timeout status (default 180s sweeper).

---

## 📦 Registered Subsystems & When to Route

| Subsystem | Transport | Capabilities & Tools |
| :--- | :--- | :--- |
| **`github`** | stdio | PRs, issues, commits, repo search, git diffs, file content |
| **`composio`** | http | Google Drive, Gmail, Google Calendar, Telegram, Linear, Notion |
| **`agent_memory`** | stdio | 4-way hybrid retrieval, Letta persona blocks, atomic facts, dream cycle |
| **`herdlink`** | stdio | Headless SSH, tmux terminal multiplexing, remote agent supervision |
| **`miku_researcher`**| stdio | SearXNG local search, Tavily synthesis, Defuddle clean markdown scraping |
| **`browser_use`** | stdio | Headless browser CDP automation, click, type, screenshot, navigation |
| **`obsidian`** | stdio | Local Obsidian vault note search, frontmatter patch, bidirectional links |
| **`context7`** | stdio | Up-to-date live library and framework API documentation lookup |
| **`ast_grep`** | stdio | Tree-sitter structural syntax search and AST-aware code rewrites |

---

## ⚡ Execution Protocol in Codemode

When operating in environments supporting `codemode`, invoke `laya-hub` tools directly:

```javascript
// 1. Inspect registered capabilities & active processes
const status = await tools.mcp__laya_hub__hub_status({});

// 2. Route an ambiguous or multi-step request
const route = await tools.mcp__laya_hub__hub_route({
  task: "Find all pull requests fixing memory leaks in repo"
});

// 3. Call matched tool on target server
const result = await tools.mcp__laya_hub__hub_call({
  server: "github",
  tool: "search_issues_and_pull_requests",
  arguments: { query: "repo:surtr85/laya-hub memory leak" }
});
```
