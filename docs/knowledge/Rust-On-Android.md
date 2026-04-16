# Rust on Android

> Google's adoption of Rust in AOSP and what it means for Peko Agent.

---

## Official Support

Google formally adopted Rust for AOSP in **April 2021**. As of Android 13:

- **~1.5 million lines** of Rust in AOSP
- **21% of new native code** is Rust
- Memory safety bugs dropped from **223 (2019) to 85 (2022)**

This means Peko Agent's language choice has direct precedent in production Android.

## Rust in Production Android

| Component | Purpose | Lines of Rust |
|---|---|---|
| Keystore2 | Cryptographic key management | ~35K |
| UWB stack | Ultra-Wideband communication | ~25K |
| DNS-over-HTTP3 | Secure DNS resolution | ~15K |
| Bluetooth (Gabeldorsche) | Bluetooth stack rewrite | ~50K+ |
| Android Virtualization Framework | pKVM + Microdroid | ~100K+ |
| Binder (Rust bindings) | IPC framework | ~10K |

## How AOSP Builds Rust

AOSP uses the **Soong** build system (not Cargo) for Rust:

```blueprint
// Android.bp
rust_binary {
    name: "peko-agent",
    srcs: ["src/main.rs"],
    edition: "2021",
    rustlibs: [
        "libpeko_core",
        "libpeko_transport",
        "libtokio",
    ],
}
```

However, Peko Agent uses **Cargo** for development and cross-compiles with the NDK. The Soong integration is only needed if you want to build as part of a full AOSP ROM. See [[Cross-Compilation]] for the Cargo approach.

## Cargo vs Soong

| Aspect | Cargo | Soong |
|---|---|---|
| Development | Standard Rust workflow | AOSP-specific |
| Dependencies | crates.io | Must vendor into AOSP tree |
| Build speed | Fast (incremental) | Slow (full AOSP build) |
| Testing | `cargo test` on host | Complex emulator setup |
| Deployment | ADB push binary | Build full ROM image |
| When to use | Development + standalone binary | Full AOSP integration |

**Recommendation**: Use Cargo for development. Only integrate with Soong if building a custom ROM.

## Rust/C Interop

Peko Agent mostly avoids C interop since it uses pure-Rust crates. The main FFI boundary is `libc` for system calls, wrapped by `nix`:

```rust
// nix provides safe wrappers
use nix::sys::ioctl::*;
use nix::fcntl::open;
use nix::sys::mman::mmap;

// Instead of raw unsafe libc calls
```

For optional Binder access, the `binder_ndk` Rust bindings (part of AOSP) provide safe wrappers.

## Why Rust for This Project

See [[Why-Rust]] for the full rationale. Summary:

1. **Memory safety** — No GC pauses, no memory leaks for a long-running daemon
2. **Async/await** — Efficient concurrency on limited mobile hardware
3. **Trait system** — Clean abstraction boundaries between crates
4. **Cross-compilation** — Straightforward NDK toolchain support
5. **Binary size** — Comparable to C, much smaller than Go/Java
6. **Google precedent** — Proven in production Android

## Related

- [[Why-Rust]] — Detailed language choice rationale
- [[Cross-Compilation]] — NDK toolchain setup
- [[../architecture/Crate-Map]] — Workspace structure
- [[../research/Related-Work-Overview]] — RustBelt and safety proofs

---

#knowledge #rust #android #aosp
