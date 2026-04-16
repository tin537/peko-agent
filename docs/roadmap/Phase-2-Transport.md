# Phase 2: Transport

> HTTP client, SSE parsing, and real LLM provider integration.

---

## Goal

Replace the mock LLM provider with real API calls. Agent runs on desktop, calls Anthropic/OpenRouter, and correctly parses streamed tool-use responses.

## Prerequisites

- [[Phase-1-Foundation]] completed
- Anthropic API key (or OpenRouter key)

## Tasks

### 2.1 SSE Parser

- [ ] Implement `SseParser` struct with `feed()` method
- [ ] Handle chunk boundary splitting
- [ ] Handle multi-line data fields
- [ ] Handle `[DONE]` sentinel (OpenAI format)
- [ ] Tests with recorded real SSE streams from Anthropic and OpenAI

See [[../implementation/SSE-Streaming]] for the design.

### 2.2 StreamEvent Types

- [ ] Define `StreamEvent` enum (all variants)
- [ ] Define `ContentBlockType`, `StopReason` enums
- [ ] Implement `Display` / `Debug` for logging

### 2.3 AnthropicProvider

- [ ] Implement `LlmProvider` trait for Anthropic
- [ ] Build request body (system, messages, tools, stream: true)
- [ ] Set correct headers (anthropic-version, x-api-key)
- [ ] Parse SSE events into `StreamEvent`s
- [ ] Handle tool use: accumulate `input_json_delta`, parse on `content_block_stop`
- [ ] Handle prompt caching (`cache_control` on system prompt blocks)
- [ ] Handle extended thinking (optional)
- [ ] Integration test with real API call

See [[../implementation/LLM-Providers]] for API details.

### 2.4 OpenAICompatProvider

- [ ] Implement `LlmProvider` for OpenAI-compatible APIs
- [ ] Handle different system prompt format (role: system message)
- [ ] Handle tool call delta accumulation (indexed by tool call position)
- [ ] Handle `data: [DONE]` stream termination
- [ ] Test with OpenRouter endpoint

### 2.5 PekoLocalProvider (optional in this phase)

- [ ] Implement ChatML prompt formatting
- [ ] Parse `<tool_call>` XML from model output
- [ ] Test with local vLLM/llama.cpp server

### 2.6 ProviderChain

- [ ] Implement priority-based failover
- [ ] Failover on: connection error, 429, 5xx
- [ ] Don't failover on: 400, 401, 403
- [ ] Test failover behavior with mock HTTP server

### 2.7 HTTP Client Configuration

- [ ] Configure `reqwest::Client` with rustls, timeouts, connection pooling
- [ ] Implement exponential backoff retry on 5xx
- [ ] Ensure client is built once and shared via `Arc`

### 2.8 Integration: Core + Transport

- [ ] Replace mock provider in AgentRuntime with real AnthropicProvider
- [ ] Run a real multi-step task: agent calls tools (using desktop mock tools like shell, file_read)
- [ ] Verify token counting against actual API usage reports
- [ ] Verify context compression triggers correctly on long conversations

## Definition of Done

A desktop integration test that:
1. Creates `AgentRuntime` with `AnthropicProvider` + desktop mock tools
2. Sends a task like "What files are in the current directory?"
3. Agent calls a `shell` mock tool with `ls`
4. Agent returns the answer
5. Full conversation persisted to SQLite

## Output Artifacts

- `crates/peko-transport/` — fully functional
- Real LLM API integration working
- Recorded SSE streams for regression testing

## Related

- [[Phase-1-Foundation]] — Previous phase
- [[Phase-3-Hardware]] — Next phase
- [[../implementation/peko-transport]] — Transport crate design
- [[../implementation/LLM-Providers]] — Provider implementations
- [[../implementation/SSE-Streaming]] — SSE parser design

---

#roadmap #phase-2 #transport
