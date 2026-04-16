# Phase 1: Foundation

> Core agent loop, types, and configuration — all on desktop.

---

## Goal

A working [[../implementation/ReAct-Loop|ReAct loop]] with mock tools, running on macOS or desktop Linux. No Android, no real LLM calls yet.

## Tasks

### 1.1 Workspace Setup

- [ ] Create Cargo workspace with all crate directories
- [ ] Configure `Cargo.toml` for each crate with placeholder dependencies
- [ ] Set up `.cargo/config.toml` for cross-compilation (will use later)
- [ ] Add `rustfmt.toml`, `.gitignore`

See [[../architecture/Crate-Map]] for the directory structure.

### 1.2 peko-config

- [ ] Define `PekoConfig` struct with all fields
- [ ] Implement TOML parsing via `serde` + `toml`
- [ ] Support environment variable overrides
- [ ] Write example `config.toml`
- [ ] Tests for parsing, env override precedence

### 1.3 Message Types (peko-core)

- [ ] Define `Message` enum: System, User, Assistant, ToolResult
- [ ] Implement serialization for Anthropic message format
- [ ] Implement serialization for OpenAI message format
- [ ] Tests for round-trip serialization

### 1.4 Tool Trait + Registry (peko-core)

- [ ] Define `Tool` trait
- [ ] Implement `ToolRegistry` with register, schemas, execute, available_tools
- [ ] Define `ToolResult` struct
- [ ] Create 2-3 mock tools for testing (echo tool, math tool, file read tool)
- [ ] Tests for registration, schema generation, dispatch, unknown tool handling

See [[../implementation/Tool-System]] for the design.

### 1.5 Iteration Budget (peko-core)

- [ ] Implement `IterationBudget` with `AtomicUsize` / `AtomicBool`
- [ ] Tests for decrement, exhaustion, interrupt from another thread

### 1.6 Context Compressor (peko-core)

- [ ] Implement approximate token counter (`len / 4`)
- [ ] Implement head-tail compression algorithm
- [ ] Tests with large conversation histories

See [[../implementation/Context-Compression]] for the algorithm.

### 1.7 Session Store (peko-core)

- [ ] Define SQLite schema (sessions + messages + FTS5)
- [ ] Implement create/append/load/search operations
- [ ] Tests for CRUD and full-text search

See [[../implementation/Session-Persistence]] for the schema.

### 1.8 System Prompt Builder (peko-core)

- [ ] Implement `SystemPrompt` struct that assembles SOUL + MEMORY + tool schemas
- [ ] Create a default SOUL.md template
- [ ] Tests for prompt assembly

### 1.9 Agent Runtime (peko-core)

- [ ] Implement `AgentRuntime` struct wiring everything together
- [ ] Implement `run_task()` with the full ReAct loop
- [ ] Use a **mock LLM provider** that returns scripted responses
- [ ] End-to-end test: runtime + mock provider + mock tools → task completion

This is the big integration piece — see [[../implementation/peko-core]].

## Definition of Done

```bash
cargo test --workspace     # All tests pass
cargo build --release      # Compiles on host (macOS/Linux)
```

And an integration test that:
1. Creates an `AgentRuntime` with mock provider and tools
2. Runs a multi-step task
3. Verifies the correct tools were called in the correct order
4. Verifies the conversation was persisted to SQLite

## Output Artifacts

- `crates/peko-core/` — fully functional
- `crates/peko-config/` — fully functional
- Mock tools for testing
- SQLite database with test data

## Related

- [[Implementation-Roadmap]] — Phase overview
- [[Phase-2-Transport]] — Next phase
- [[../implementation/peko-core]] — Core crate design
- [[../implementation/Tool-System]] — Tool architecture

---

#roadmap #phase-1 #foundation
