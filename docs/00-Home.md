# Peko Agent — Knowledge Base

> **An agent-as-OS architecture in Rust for frameworkless Android.**
> Single Rust binary. PID-1 child. Direct kernel access. No framework. No Zygote. No ART.

---

## Map of Content

### Vision & Architecture
- [[01-Vision]] — Why Agent-as-OS? The core thesis
- [[architecture/Architecture-Overview]] — System design at a glance
- [[architecture/Boot-Sequence]] — From power-on to agent loop
- [[architecture/Crate-Map]] — Cargo workspace structure and dependency graph
- [[architecture/Curiosity-Learning-Backbone]] — Novelty / surprise / risk / policy / world model / intrinsic reward (design)

### Core Systems
- [[implementation/peko-core]] — Agent runtime and orchestration
- [[implementation/peko-transport]] — LLM API communication
- [[implementation/peko-tools-android]] — Android tool implementations
- [[implementation/peko-hal]] — Hardware abstraction layer
- [[implementation/peko-config]] — Configuration system
- [[implementation/peko-agent-binary]] — Final binary entry point

### Deep Dives
- [[implementation/ReAct-Loop]] — The observe-think-act cycle
- [[implementation/Tool-System]] — Trait-based tool architecture
- [[implementation/Context-Compression]] — Managing token budgets
- [[implementation/LLM-Providers]] — Anthropic, OpenAI, local models
- [[implementation/SSE-Streaming]] — Real-time stream parsing
- [[implementation/Session-Persistence]] — SQLite + FTS5 storage

### Knowledge Prerequisites
- [[knowledge/Android-Internals]] — init, Zygote, framework
- [[knowledge/Linux-Kernel-Interfaces]] — evdev, framebuffer, DRM, serial
- [[knowledge/SELinux-Policy]] — Mandatory access control
- [[knowledge/Touch-Input-System]] — Input event injection
- [[knowledge/Screen-Capture]] — Framebuffer and DRM reads
- [[knowledge/Telephony-AT-Commands]] — Modem communication
- [[knowledge/Cross-Compilation]] — NDK toolchain setup
- [[knowledge/Rust-On-Android]] — Rust in the AOSP ecosystem
- [[knowledge/Why-Rust]] — Language choice rationale

### Research & References
- [[research/Related-Work-Overview]] — Academic landscape
- [[research/Mobile-Agents]] — AppAgent, Mobile-Agent, AutoDroid
- [[research/Computer-Use-Agents]] — Claude CU, OSWorld, CogAgent
- [[research/ReAct-Paper]] — Reasoning + Acting paradigm
- [[research/Agent-Benchmarks]] — AgentBench, AndroidWorld, AITW

### Roadmap & Execution
- [[roadmap/Implementation-Roadmap]] — Phased build plan
- [[roadmap/Phase-1-Foundation]] — Core + config + types
- [[roadmap/Phase-2-Transport]] — HTTP + SSE + providers
- [[roadmap/Phase-3-Hardware]] — HAL + kernel interfaces
- [[roadmap/Phase-4-Tools]] — Android tool suite
- [[roadmap/Phase-5-Integration]] — Binary + socket + signals
- [[roadmap/Phase-6-Android-Deploy]] — Device deployment
- [[roadmap/Testing-Strategy]] — Testing at every layer
- [[roadmap/Device-Requirements]] — Hardware needs

### Gap Closure (vs Original Peko Agent)
- [[roadmap/Gap-Analysis]] — Full comparison with ftmstars Peko Agent
- [[roadmap/Phase-7-Learning-Loop]] — Memory + Skills + User Model
- [[roadmap/Phase-8-Messaging]] — Telegram / Discord / WhatsApp gateway
- [[roadmap/Phase-9-Scheduler-MCP]] — Cron scheduler + MCP + Subagents

### Possibilities & Frontiers
- [[roadmap/Challenges-And-Risks]] — What could go wrong
- [[roadmap/Possibilities]] — Where this leads next
- [[Glossary]] — Terms and definitions

---

## Quick Stats (Design Targets)

| Metric | Target |
|---|---|
| Binary size | < 15 MB |
| Memory overhead | < 50 MB (vs ~800 MB with framework) |
| Boot to agent loop | < 3 seconds after kernel |
| Max iterations per task | 50 (configurable) |
| Supported providers | Anthropic, OpenRouter, Local models |
| Implementation estimate | ~5,000 lines of Rust |

---

#MOC #peko-agent
