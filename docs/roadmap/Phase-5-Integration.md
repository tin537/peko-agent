# Phase 5: Integration

> Assembling the final binary with control socket and signal handling.

---

## Goal

A single `peko-agent` binary that runs as a standalone daemon on Android, accepting tasks via Unix socket and handling system signals.

## Prerequisites

- [[Phase-1-Foundation]] through [[Phase-4-Tools]] completed

## Tasks

### 5.1 Binary Entry Point

- [ ] Create `src/main.rs` with tokio runtime initialization
- [ ] Wire up config loading → tool registration → provider creation → runtime
- [ ] Implement `register_android_tools()` helper with hardware auto-detection
- [ ] Implement `build_provider_chain()` from config
- [ ] Graceful error handling on startup failures

### 5.2 Control Socket

- [ ] Implement Unix domain socket listener at configurable path
- [ ] Define JSON-RPC protocol: `run_task`, `status`, `interrupt`, `shutdown`
- [ ] Handle concurrent connections (one active task, queue or reject others)
- [ ] Return structured responses with status, response text, iteration count
- [ ] Test via `socat` or `nc` over ADB

See [[../implementation/peko-agent-binary]] for the protocol.

### 5.3 Signal Handling

- [ ] Handle `SIGTERM` via `tokio::signal` → interrupt agent loop, clean shutdown
- [ ] Handle `SIGHUP` → reload configuration from disk
- [ ] Ensure signal handlers are async-safe

### 5.4 Logging

- [ ] Set up `tracing` with two subscribers:
  - Android logcat output (via `__android_log_write` FFI)
  - File log at `/data/peko/peko.log`
- [ ] Configurable log level from config.toml
- [ ] Structured logging with span context (session_id, tool_name, etc.)

### 5.5 Dangerous Tool Confirmation

- [ ] When a dangerous tool is invoked, send confirmation request to control socket
- [ ] Wait for approval/denial from the socket client
- [ ] Timeout after configurable duration → deny by default
- [ ] Test: trigger SMS tool → verify confirmation prompt appears

### 5.6 Startup Task

- [ ] If `[startup].task` is configured, run it immediately after initialization
- [ ] Log results to session store
- [ ] Handle startup task failure gracefully

### 5.7 Full Binary Build

- [ ] Configure release profile (LTO, strip, size optimization)
- [ ] Cross-compile for `aarch64-linux-android`
- [ ] Verify binary size < 15 MB
- [ ] Push to device, run manually, verify all features work

## Testing

### Manual Testing

```bash
# Build
cargo build --target aarch64-linux-android --release

# Deploy
adb push target/aarch64-linux-android/release/peko-agent /data/local/tmp/
adb push config.toml /data/peko/config.toml
adb shell su -c chmod 755 /data/local/tmp/peko-agent

# Run
adb shell su -c /data/local/tmp/peko-agent --config /data/peko/config.toml &

# Send a task
echo '{"jsonrpc":"2.0","method":"run_task","params":{"input":"Take a screenshot and describe what you see"},"id":1}' | \
  adb shell su -c socat - UNIX-CONNECT:/dev/socket/peko

# Check status
echo '{"jsonrpc":"2.0","method":"status","id":2}' | \
  adb shell su -c socat - UNIX-CONNECT:/dev/socket/peko

# Interrupt
echo '{"jsonrpc":"2.0","method":"interrupt","id":3}' | \
  adb shell su -c socat - UNIX-CONNECT:/dev/socket/peko
```

## Definition of Done

Binary running on device as a standalone process (not yet via init):
1. Accepts JSON-RPC commands via Unix socket
2. Executes multi-step tasks with real tools
3. Handles SIGTERM gracefully
4. Logs to both logcat and file
5. Binary size < 15 MB

## Related

- [[Phase-4-Tools]] — Previous phase
- [[Phase-6-Android-Deploy]] — Next phase (init.rc integration)
- [[../implementation/peko-agent-binary]] — Binary design
- [[../architecture/Architecture-Overview]] — System diagram

---

#roadmap #phase-5 #integration #binary
