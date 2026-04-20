# peko-agent (binary)

> The final executable that ties everything together.

---

## Purpose

`peko-agent` is the binary crate — the `main()` function. It wires up all the library crates, opens the control socket, handles signals, and enters the main event loop.

## Startup Sequence

```rust
#[tokio::main]
async fn main() -> Result<()> {
    // 1. Parse CLI args + load config
    let config = PekoConfig::load("--config path")?;

    // 2. Initialize logging (tracing → logcat + file)
    init_logging(&config)?;

    // 3. Create tool registry with Android tools
    let mut registry = ToolRegistry::new();
    register_android_tools(&mut registry, &config)?;

    // 4. Create LLM provider chain
    let provider = build_provider_chain(&config)?;

    // 5. Create agent runtime
    let mut runtime = AgentRuntime::new(
        config.agent, Arc::new(registry), provider
    ).await?;

    // 6. Open control socket
    let socket = UnixListener::bind("/dev/socket/peko")?;

    // 7. Handle signals
    let budget = runtime.budget_handle(); // Arc clone for interrupt
    tokio::spawn(async move {
        signal(SIGTERM).await; // init sends this on shutdown
        budget.interrupt();
    });

    // 8. Optional startup task
    if let Some(task) = config.startup.task {
        runtime.run_task(&task).await?;
    }

    // 9. Main event loop — accept commands on socket
    loop {
        let (stream, _) = socket.accept().await?;
        handle_connection(&mut runtime, stream).await?;
    }
}
```

## Control Socket Protocol

The Unix domain socket at `/dev/socket/peko` accepts **JSON-RPC** commands:

### Run a task

```json
→ {"jsonrpc": "2.0", "method": "run_task", "params": {"input": "Send an SMS to +1234567890 saying hello"}, "id": 1}
← {"jsonrpc": "2.0", "result": {"status": "completed", "response": "SMS sent successfully", "iterations": 3}, "id": 1}
```

### Query status

```json
→ {"jsonrpc": "2.0", "method": "status", "id": 2}
← {"jsonrpc": "2.0", "result": {"state": "idle", "iterations_remaining": 47, "session_id": "abc123"}, "id": 2}
```

### Interrupt current task

```json
→ {"jsonrpc": "2.0", "method": "interrupt", "id": 3}
← {"jsonrpc": "2.0", "result": {"interrupted": true}, "id": 3}
```

### Shutdown

```json
→ {"jsonrpc": "2.0", "method": "shutdown", "id": 4}
← {"jsonrpc": "2.0", "result": {"shutting_down": true}, "id": 4}
```

## Signal Handling

| Signal | Source | Action |
|---|---|---|
| `SIGTERM` | init process (system shutdown) | Interrupt agent loop, clean up, exit |
| `SIGHUP` | Manual | Reload configuration from disk |

Both handled via `tokio::signal` for async-compatible signal processing.

## Logging

Dual output via the `tracing` crate:

1. **logcat** — via `__android_log_write()` for integration with Android's logging system
2. **File log** — `/data/peko/peko.log` for persistent debugging

Log levels configurable in [[peko-config|config.toml]].

## External Control

Who talks to the control socket:

- **ADB shell** — for development: `echo '{"method":"run_task",...}' | nc -U /dev/socket/peko`
- **Companion app** — a minimal Android app (if framework is running) that sends commands
- **Another init service** — for automated task scheduling
- **SSH** — for remote control over network

## Dependencies

```toml
[dependencies]
peko-core = { path = "crates/peko-core" }
peko-transport = { path = "crates/peko-transport" }
peko-tools-android = { path = "crates/peko-tools-android" }
peko-hal = { path = "crates/peko-hal" }
peko-config = { path = "crates/peko-config" }
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
tracing-subscriber = "0.3"
serde_json = "1"
```

## Build Output

Target: `aarch64-linux-android` ELF binary at `/system/bin/peko-agent`.

See [[../knowledge/Cross-Compilation]] for the NDK toolchain setup.

## Related

- [[../architecture/Boot-Sequence]] — How init launches this binary
- [[peko-core]] — The agent runtime this binary creates
- [[peko-config]] — Configuration loading
- [[../architecture/Crate-Map]] — Top of the dependency tree

---

#implementation #binary #main
