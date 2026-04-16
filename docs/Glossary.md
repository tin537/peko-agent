# Glossary

## A

**ART (Android Runtime)** — The managed runtime that executes Android apps. Replaced Dalvik in Android 5.0. Peko Agent bypasses this entirely. See [[knowledge/Android-Internals]].

**AT Commands** — Hayes command set for controlling modems. Used by [[knowledge/Telephony-AT-Commands]] for SMS and calls.

**AgentBench** — Benchmark for evaluating LLM agents across 8 environments. See [[research/Agent-Benchmarks]].

## B

**Binder IPC** — Android's inter-process communication mechanism. The framework routes all hardware access through Binder. Peko Agent eliminates this overhead. See [[knowledge/Android-Internals]].

## C

**ChatML** — Chat Markup Language format used by ChatML models (`<|im_start|>` / `<|im_end|>`). See [[implementation/LLM-Providers]].

**Context Compression** — Strategy for keeping conversation within token limits. See [[implementation/Context-Compression]].

**CogAgent** — 18B parameter visual language model for GUI grounding. See [[research/Computer-Use-Agents]].

## D

**DRM/KMS** — Direct Rendering Manager / Kernel Mode Setting. Modern Linux display interface. See [[knowledge/Screen-Capture]].

## E

**evdev** — Linux event device interface for input. See [[knowledge/Touch-Input-System]].

## F

**Framebuffer** — Legacy Linux display interface at `/dev/graphics/fb0`. See [[knowledge/Screen-Capture]].

**FTS5** — SQLite full-text search extension. Used by [[implementation/Session-Persistence]].

## H

**HAL** — Hardware Abstraction Layer. In Peko Agent, [[implementation/peko-hal]] wraps kernel ioctls. Different from Android's HAL (which uses Binder).

## I

**init.rc** — Android's init configuration script language. See [[architecture/Boot-Sequence]].

**Iteration Budget** — Configurable limit on agent loop cycles per task. See [[implementation/ReAct-Loop]].

## L

**LLM Provider** — Abstraction over different LLM API backends. See [[implementation/LLM-Providers]].

## N

**NDK** — Android Native Development Kit. Provides the cross-compilation toolchain. See [[knowledge/Cross-Compilation]].

## P

**PID 1** — Process ID 1, the init process. Peko Agent runs as a direct child. See [[architecture/Boot-Sequence]].

## R

**ReAct** — Reasoning + Acting paradigm. The agent loop pattern. See [[implementation/ReAct-Loop]] and [[research/ReAct-Paper]].

**RIL** — Radio Interface Layer. Android's telephony abstraction that Peko Agent bypasses via direct [[knowledge/Telephony-AT-Commands|AT commands]].

## S

**SELinux** — Security-Enhanced Linux. Mandatory access control. See [[knowledge/SELinux-Policy]].

**SSE** — Server-Sent Events. HTTP streaming protocol used by LLM APIs. See [[implementation/SSE-Streaming]].

**SoM (Set-of-Mark)** — Visual prompting technique overlaying markers on UI elements. See [[research/Computer-Use-Agents]].

**Soong** — Android's build system (replaces Make). See [[knowledge/Rust-On-Android]].

## T

**Tool Registry** — Dynamic tool registration and dispatch system. See [[implementation/Tool-System]].

**tokio** — Async runtime for Rust. Core dependency. See [[knowledge/Why-Rust]].

## U

**uinput** — Linux kernel module for creating virtual input devices. See [[knowledge/Touch-Input-System]].

## Z

**Zygote** — Android process that forks to create all app processes. Peko Agent boots before it. See [[knowledge/Android-Internals]].

---

#glossary #reference
