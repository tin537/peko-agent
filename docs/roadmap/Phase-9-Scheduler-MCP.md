# Phase 9: Scheduler + MCP + Subagents

> Autonomous recurring tasks, dynamic tool extension, and parallel work.

---

## 9a: Cron Scheduler (~3-5 days)

### What it does

Define recurring tasks that the agent executes automatically.
Results delivered to web UI, Telegram, or stored in sessions.

### Config

```toml
[[schedule]]
name = "morning_check"
cron = "0 8 * * *"            # Every day at 8 AM
task = "Check battery level, WiFi status, and any new SMS messages. Summarize."
notify = "telegram"           # or "web" or "log"

[[schedule]]
name = "hourly_screenshot"
cron = "0 * * * *"
task = "Take a screenshot and save to /data/peko/screenshots/"
notify = "log"
```

### Implementation

- Lightweight cron parser (no external crate needed for basic patterns)
- Background tokio task checks schedule every 60 seconds
- Executes task via AgentRuntime, stores result in session
- Optional notification delivery

### Tasks

- [ ] Implement simple cron expression parser (minute, hour, day, month, weekday)
- [ ] Add schedule config to PekoConfig
- [ ] Background scheduler task in main.rs
- [ ] `/api/schedule` endpoint (list, add, remove, trigger)
- [ ] Schedule section in web UI

---

## 9b: MCP Integration (~1-2 weeks)

### What it does

Connect to [Model Context Protocol](https://modelcontextprotocol.io/) servers
to dynamically extend the agent's tools without recompiling.

### Architecture

```
peko-agent
  ├── Built-in tools (screenshot, touch, shell, ...)
  └── MCP Bridge
      ├── MCP Server 1 (file search)
      ├── MCP Server 2 (web browse)
      └── MCP Server 3 (custom tools)
```

### Implementation

- MCP client speaks JSON-RPC over stdio or HTTP
- Discover tools from MCP server → register as dynamic tools
- Tool schemas from MCP → JSON Schema for LLM
- Tool calls routed to MCP server, results returned

### Config

```toml
[[mcp.servers]]
name = "web-tools"
command = "npx"
args = ["-y", "@anthropic/mcp-web-tools"]

[[mcp.servers]]
name = "custom"
url = "http://localhost:3001"
```

### Tasks

- [ ] MCP client (JSON-RPC over stdio + HTTP)
- [ ] Tool discovery from MCP servers
- [ ] Dynamic tool registration into ToolRegistry
- [ ] Config: mcp.servers list
- [ ] MCP tools visible in Device tab

---

## 9c: Subagent Delegation (~1 week)

### What it does

Spawn isolated agent instances for parallel work. The main agent
can delegate subtasks to child agents.

### Architecture

```
Main Agent (task: "Research X and send summary to Y")
  ├── Subagent A: "Research X" (parallel)
  └── Subagent B: "Send summary to Y" (waits for A)
```

### Implementation

- New tool: `delegate(task, wait=true/false)`
- Spawns a new AgentRuntime with its own session
- Shares the same ToolRegistry and provider
- Returns result to parent agent

### Tasks

- [ ] `DelegateTool` — spawns a child AgentRuntime
- [ ] Shared tool registry (Arc<ToolRegistry>)
- [ ] Result aggregation back to parent
- [ ] Budget limits for subagents (prevent runaway)

---

#roadmap #phase-9 #scheduler #mcp #subagents
