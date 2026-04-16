# Why Rust

> Language choice rationale for an always-on agent daemon.

---

## The Constraints

Peko Agent runs as a **PID-1 child daemon** on Android. This imposes hard requirements:

| Constraint | Why |
|---|---|
| No memory leaks | Daemon runs indefinitely — leaks accumulate |
| No GC pauses | Real-time responsiveness to hardware events |
| Small binary | Limited `/system` partition space |
| Low memory | Mobile device, ~2-4 GB total RAM |
| Safe concurrency | Multiple async tasks (SSE stream, tools, socket, signals) |
| Cross-compilation | Must target `aarch64-linux-android` |

## Language Comparison

| Language | Memory safety | No GC | Binary size | Android support | Async | Verdict |
|---|---|---|---|---|---|---|
| **Rust** | Yes (compile-time) | Yes | ~8 MB | NDK + AOSP | tokio | **Best fit** |
| C | No | Yes | ~5 MB | NDK | manual/libevent | Unsafe, slow development |
| C++ | Partial (smart ptrs) | Yes | ~8 MB | NDK | coroutines (C++20) | Complexity, undefined behavior risk |
| Go | Yes (runtime) | GC pauses | ~15 MB | gomobile | goroutines | GC pauses, larger binary |
| Java/Kotlin | Yes (runtime) | GC pauses | N/A | Native | coroutines | Requires ART — defeats the purpose |
| Python | Yes (runtime) | GC | ~50 MB+ | Chaquopy | asyncio | Too heavy, too slow |
| Zig | Partial | Yes | ~5 MB | Cross-compile | io_uring | Immature ecosystem |

## Rust's Three Critical Properties

### 1. Memory Safety Without Garbage Collection

Rust's ownership system prevents:
- Use-after-free
- Double-free
- Buffer overflows
- Data races

All at **compile time**, with zero runtime overhead. This means:
- The daemon can run for weeks without memory issues
- No GC pauses that could delay hardware interaction
- `unsafe` blocks are limited to kernel ioctl calls (wrapped by `nix`)

Academic proof: **RustBelt** (Jung et al., POPL 2018) machine-checked Rust's safety guarantees for `Arc`, `Mutex`, and other standard library types.

### 2. Async/Await on Minimal Hardware

Tokio provides:
- Multi-threaded async executor
- Efficient I/O (epoll on Linux)
- Timer management
- Signal handling

All running on 2-4 cores typical of mobile devices. The agent handles concurrent tasks:

```
tokio runtime
├── Task 1: SSE stream processing (provider.stream_completion())
├── Task 2: Tool execution (potentially blocking I/O)
├── Task 3: Control socket listener
└── Task 4: Signal handler (SIGTERM, SIGHUP)
```

Without async, you'd need threads for each — expensive on mobile. With tokio, they share a small thread pool.

### 3. Trait System for Clean Architecture

Rust traits enable the [[../architecture/Crate-Map|crate boundary]] design:

```rust
// peko-core defines the interface
pub trait Tool: Send + Sync + 'static { ... }
pub trait LlmProvider: Send + Sync { ... }

// peko-tools-android implements it
impl Tool for TouchTool { ... }

// peko-transport implements it
impl LlmProvider for AnthropicProvider { ... }

// peko-agent wires them together
let runtime = AgentRuntime::new(config, tools, provider);
```

This makes `peko-core` **fully testable on desktop** — swap in mock implementations without touching agent logic.

## Overhead Budget

"Safe Systems Programming in Rust" (Balasubramanian et al., HotOS 2017) measured Rust's overhead:

- **Zero-cost abstractions**: trait dispatch via vtable ≈ C++ virtual call
- **Bounds checking**: ~2-5% overhead (eliminable with `unsafe` where profiled)
- **Software fault isolation**: 90 cycles per protected method call

For an agent that spends 99%+ of its time waiting on network I/O, these overheads are negligible.

## The `unsafe` Budget

Peko Agent's `unsafe` usage is concentrated in [[../implementation/peko-hal|peko-hal]]:

| Where | Why |
|---|---|
| `ioctl()` calls | Kernel interface requires raw syscalls |
| `mmap()` for framebuffer | Memory-mapped I/O |
| `input_event` writes | Fixed-layout C struct to kernel |

All wrapped in safe Rust APIs. The rest of the codebase (core, transport, config, tools) is 100% safe Rust.

## Related

- [[Rust-On-Android]] — Google's adoption in AOSP
- [[Cross-Compilation]] — NDK toolchain
- [[../architecture/Crate-Map]] — How traits enable the workspace structure
- [[../research/Related-Work-Overview]] — RustBelt safety proofs

---

#knowledge #rust #language #rationale
