# Testing Strategy

> Testing at every layer, from unit tests to full device integration.

---

## Testing Pyramid

```
         /\
        /  \
       / E2E\          End-to-end on device
      / tests \         (Phase 5-6)
     /──────────\
    / Integration\      Core + transport + mock tools
   /    tests     \     (Phase 2)
  /────────────────\
 /    Unit tests    \   Each module in isolation
/____________________\  (Phase 1-4)
```

## Layer 1: Unit Tests (All Phases)

Run on host (macOS/Linux), no device needed.

### peko-core

| Module | Test coverage |
|---|---|
| `message.rs` | Serialization round-trips for all Message variants |
| `tool.rs` | Registry: register, duplicate name, unknown tool, schema generation |
| `budget.rs` | Decrement, exhaustion, interrupt from another thread |
| `compressor.rs` | Token counting, compression trigger threshold, head-tail preservation |
| `session.rs` | CRUD operations, FTS5 search, concurrent access |
| `prompt.rs` | Prompt assembly with varying components |

### peko-transport

| Module | Test coverage |
|---|---|
| `sse.rs` | Chunked parsing, multi-line data, incomplete events, `[DONE]` |
| `anthropic.rs` | Event mapping from recorded SSE streams |
| `openai.rs` | Delta accumulation, tool call indexing |
| `chain.rs` | Failover on 429/5xx, no failover on 400/401 |

### peko-config

| Module | Test coverage |
|---|---|
| `lib.rs` | TOML parsing, env var override, missing fields, invalid values |

### peko-hal (cross-compiled unit tests)

| Module | Test coverage |
|---|---|
| `input.rs` | Event struct layout, coordinate scaling math |
| `framebuffer.rs` | Pixel format decoding (RGBA, BGRA, RGB565) |
| `modem.rs` | AT command formatting, response parsing |

## Layer 2: Integration Tests (Phase 2+)

### Mock Provider Tests

```rust
#[tokio::test]
async fn test_react_loop_with_mock() {
    let mock_provider = MockProvider::new(vec![
        // First call: LLM requests a tool
        mock_response(tool_call("shell", json!({"command": "ls"}))),
        // Second call: LLM gives final answer
        mock_response(text("The directory contains: file1.txt, file2.txt")),
    ]);

    let mut registry = ToolRegistry::new();
    registry.register(MockShellTool::new("file1.txt\nfile2.txt"));

    let mut runtime = AgentRuntime::new(config, Arc::new(registry), mock_provider).await?;
    let result = runtime.run_task("List the files").await?;

    assert_eq!(result.iterations, 2);
    assert!(result.text.contains("file1.txt"));
}
```

### Recorded SSE Stream Tests

Capture real API responses and replay them:

```rust
#[test]
fn test_anthropic_sse_parsing() {
    let recorded = include_str!("fixtures/anthropic_tool_use.sse");
    let events = parse_recorded_stream(recorded);

    assert!(events.iter().any(|e| matches!(e, StreamEvent::ToolUseStart { name, .. } if name == "screenshot")));
}
```

## Layer 3: Device Tests (Phase 3+)

Standalone test binaries pushed to device:

```bash
# Test input injection
cargo build --target aarch64-linux-android --example test_touch
adb push target/.../test_touch /data/local/tmp/
adb shell su -c /data/local/tmp/test_touch
# Expected: visible tap on screen

# Test screen capture
cargo build --target aarch64-linux-android --example test_screenshot
adb push target/.../test_screenshot /data/local/tmp/
adb shell su -c /data/local/tmp/test_screenshot
adb pull /data/local/tmp/screenshot.png
# Expected: valid PNG file
```

## Layer 4: End-to-End Tests (Phase 5-6)

Full agent tasks on real device with real LLM:

### Test Suite

| Test | Task | Success criteria |
|---|---|---|
| Vision | "Describe the screen" | Agent takes screenshot, returns coherent description |
| Navigation | "Open Settings" | Agent navigates to Settings app via touch |
| Multi-step | "Find WiFi settings" | Agent screenshots → taps Settings → scrolls → finds WiFi |
| SMS | "Send SMS test to +XXX" | SMS arrives at target number |
| Shell | "What's the device model?" | Agent runs `getprop` and reports correctly |
| Budget | Run a task with budget=3 | Agent stops after 3 iterations |
| Interrupt | Start task, then interrupt | Agent stops cleanly, returns partial result |

### Regression Tests

After any code change:
1. `cargo test --workspace` on host
2. Build for Android
3. Run vision test (screenshot + describe)
4. Run navigation test (open Settings)

## Mocking Strategy

| Real component | Mock replacement | Used in |
|---|---|---|
| LLM API | `MockProvider` (scripted responses) | Unit + integration tests |
| Android tools | `MockTool` (canned results) | Core integration tests |
| Framebuffer | Test image file | HAL unit tests |
| Modem | `MockSerial` (scripted AT responses) | Tool unit tests |
| Control socket | Direct function calls | Integration tests |

## CI Pipeline (Recommended)

```
On every push:
  1. cargo fmt --check
  2. cargo clippy --workspace
  3. cargo test --workspace           (host tests)
  4. cargo build --target aarch64-linux-android --release  (cross-compile check)

Nightly / manual:
  5. Deploy to test device
  6. Run device test suite
  7. Run E2E agent tests
```

## Related

- [[Implementation-Roadmap]] — When each test layer is introduced
- [[Device-Requirements]] — Test device needs
- [[../architecture/Crate-Map]] — What gets tested where

---

#roadmap #testing #quality
