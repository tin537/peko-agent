# Related Work Overview

> Academic landscape positioning Peko Agent.

---

## Where Peko Agent Sits

```
                    ┌─────────────────────┐
                    │  Peko Agent      │
                    │  (Agent-as-OS)      │
                    └────────┬────────────┘
                             │
         ┌───────────────────┼───────────────────┐
         │                   │                   │
    ┌────▼────┐        ┌─────▼─────┐       ┌────▼────┐
    │ Mobile  │        │ OS-Level  │       │ Rust    │
    │ Agents  │        │ Agents    │       │ Systems │
    └────┬────┘        └─────┬─────┘       └────┬────┘
         │                   │                   │
    ┌────▼────┐        ┌─────▼─────┐       ┌────▼────┐
    │AppAgent │        │OS-Copilot │       │AOSP Rust│
    │Mobile-  │        │Claude CU  │       │RustBelt │
    │Agent    │        │           │       │Rust-for-│
    │AutoDroid│        │           │       │Linux    │
    └─────────┘        └───────────┘       └─────────┘
```

## Research Pillars

### 1. Agent Foundations
- [[ReAct-Paper]] — The reasoning + acting paradigm
- **Toolformer** — Self-supervised tool use learning
- [[Agent-Benchmarks]] — Evaluation frameworks

### 2. Mobile Device Agents
- [[Mobile-Agents]] — AppAgent, Mobile-Agent, AutoDroid, DigiRL
- **AndroidWorld** / **AITW** — Evaluation environments and datasets

### 3. Computer Use Agents
- [[Computer-Use-Agents]] — Claude CU, CogAgent, SeeAct, OSWorld

### 4. Systems Foundation
- **RustBelt** (POPL 2018) — Machine-checked safety proof for Rust
- **Rust for Embedded** (Sharma et al., 2023) — FFI and RTOS integration
- **Binder Security** (Feng & Shin, ACSAC 2016) — Android IPC attack surface
- **SEAndroid** (Shabtai et al., 2009) — SELinux for Android

## Key Gaps Peko Agent Fills

| Existing work | Limitation | Peko Agent's answer |
|---|---|---|
| AppAgent, Mobile-Agent | Runs inside Android framework | Runs below the framework |
| AutoDroid | Depends on accessibility service | Direct kernel input injection |
| Claude Computer Use | Desktop-only, requires client tooling | Native Android binary |
| OS-Copilot | Works within a standard OS | Replaces the standard OS layer |
| Android Rust adoption | System components only | Entire agent as system process |

## The Novel Contribution

Not any single component:
- ReAct loops — well understood
- SSE parsing — standard
- Android input injection — documented
- Rust on Android — Google-proven

**The novelty is their composition** into a single binary that replaces the traditional OS application stack. This "Agent-as-OS" paradigm is new.

## Further Reading

- [[Mobile-Agents]] — Detailed survey of mobile agent architectures
- [[Computer-Use-Agents]] — Desktop and visual grounding agents
- [[ReAct-Paper]] — The foundational agent paradigm
- [[Agent-Benchmarks]] — How agents are evaluated

---

#research #overview #related-work
