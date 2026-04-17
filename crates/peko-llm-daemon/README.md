# peko-llm-daemon

Small C++17 daemon that serves an OpenAI-compatible Chat Completions API over a **Unix Domain Socket**, backed by **llama.cpp**. Designed to run alongside `peko-agent` on Android — the agent connects via UDS (no TCP, no network stack, no HTTP server exposed outside the process).

```
peko-agent (Rust)  ─ HTTP/1.1 ─▶  @peko-llm (abstract UDS)  ─▶  peko-llm-daemon (C++)
                                                                  └─ llama.cpp  (CPU / Vulkan)
                                                                     └─ Qwen3 / Gemma / Llama
```

## Why a separate process

| | This daemon | FFI / in-process |
|---|---|---|
| Isolation | GPU driver crash doesn't kill the agent | Whole agent dies |
| Language fit | C++ ↔ C++ (llama.cpp is native C++) | Rust ↔ C FFI wrappers |
| Hot-swap | Update binary without touching agent | Rebuild everything |
| Memory | Separate address space | Shared — OOM kills both |
| Debuggability | `tail daemon.log`, `strace`, `gdb` | Mixed Rust+C++ debugging |

## Endpoints

| Method | Path | Notes |
|--------|------|-------|
| `GET`  | `/health` | `{"loaded":true,"model":"…","status":"ok","version":"…"}` |
| `GET`  | `/v1/models` | OpenAI-compatible model list (always reports the currently loaded model) |
| `POST` | `/v1/chat/completions` | OpenAI Chat Completions. Supports `stream:true` (SSE with `data: {...}\n\n` chunks) and `stream:false` (single JSON response) |

Request body supports `messages`, `model` (ignored — we serve whatever is loaded), `stream`, `max_tokens`, `temperature`, `top_p`.

## Transport: abstract Unix Domain Socket

On Android, SELinux denies shell-user AF_UNIX socket **file** creation under `/data/local/tmp/`. The daemon defaults to the Linux abstract namespace (socket path starts with `@`) which lives in kernel space, bypasses filesystem ACLs, and is auto-cleaned on process exit.

```bash
# Daemon binds @peko-llm (actually \0peko-llm in the kernel)
./peko-llm-daemon --socket "@peko-llm" ...

# Test from host via adb forward
adb forward tcp:9900 localabstract:peko-llm
curl http://127.0.0.1:9900/health

# Inside the device, another process can `connect()` to the same abstract name
```

Filesystem paths also work on hosts without SELinux hassle:
```bash
./peko-llm-daemon --socket /tmp/peko-llm.sock ...
curl --unix-socket /tmp/peko-llm.sock http://localhost/health
```

## File layout

```
crates/peko-llm-daemon/
├── CMakeLists.txt            # FetchContent llama.cpp; optional GGML_VULKAN
├── build-android.sh          # NDK cross-compile (glslc autodetected)
├── deploy-test.sh            # adb push + smoke test
├── src/
│   ├── main.cpp              # args, signal handling, bootstrap
│   ├── config.h              # compile-time defaults
│   ├── openai_api.{h,cpp}    # request/response JSON + SSE helpers
│   ├── chat_template.{h,cpp} # Gemma / Qwen / Llama3 / Generic
│   ├── http_server.{h,cpp}   # cpp-httplib bound to UDS
│   └── llm_session.{h,cpp}   # llama.cpp RAII wrapper + generate loop
└── third_party/
    ├── httplib.h             # cpp-httplib v0.18.7 (MIT)
    └── json.hpp              # nlohmann/json v3.11.3 (MIT)
```

## Build (Android ARM64)

```bash
./build-android.sh             # CPU + (default ON) Vulkan
VULKAN=OFF ./build-android.sh  # CPU only (much faster to build)
```

The script auto-detects:
- NDK location (`ANDROID_NDK_HOME` env var or `~/Library/Android/sdk/ndk/*`)
- `glslc` (under `$NDK/shader-tools/<host>/`)
- CMake (prefers Android SDK's bundled)

Output: `build-android-arm64-v8a/peko-llm-daemon` (~3 MB CPU-only, ~8 MB with Vulkan).

## Build (host — for dev/testing)

```bash
cmake -S . -B build-host -DPEKO_VULKAN=OFF
cmake --build build-host -j 8
./build-host/peko-llm-daemon --model /path/to/model.gguf --socket /tmp/peko.sock
```

## CLI options

```
--model PATH            path to .gguf model file (required)
--socket PATH           UDS path (default: /data/local/tmp/peko/llm.sock)
                        use "@name" for Linux abstract namespace
--model-name NAME       advertised in /v1/models (default: filename stem)
--context N             context window in tokens (default: 2048)
--max-tokens N          max generation length (default: 512)
--temperature F         sampling temperature (default: 0.7)
--top-p F               nucleus sampling (default: 0.9)
--template NAME         gemma | qwen | llama3 | generic | auto (default: auto)
--help                  show usage
```

## Deployment flow

```bash
# 1) Build for Android
VULKAN=OFF ./build-android.sh

# 2) Push binary + model
adb push build-android-arm64-v8a/peko-llm-daemon /data/local/tmp/peko/
adb push models/qwen3-0.6b-q4_k_m.gguf /data/local/tmp/peko/models/
adb shell chmod 755 /data/local/tmp/peko/peko-llm-daemon

# 3) Start daemon (abstract socket, SELinux-safe)
adb shell "cd /data/local/tmp/peko && \
    ./peko-llm-daemon --model models/qwen3-0.6b-q4_k_m.gguf \
                      --socket @peko-llm \
                      --template qwen &"

# 4) peko-agent config points at the daemon
# config.android.toml:
# [provider.local]
# base_url = "unix://@peko-llm"
```

## Measured performance

| Host | Model | Short prompt decode | Long prompt prefill |
|------|-------|---------------------|--------------------|
| Android emulator on M-series Mac (HVF) | Qwen3-1.7B Q4_K_M | **53 tok/s** | ~90 tok/s |
| Snapdragon 680 (Redmi 10C) | Qwen3-1.7B Q4_K_M | ~3-6 tok/s (est.) | ~20 tok/s (est.) |
| Snapdragon 680 (Redmi 10C) | Qwen3-0.6B Q4_K_M | ~10-15 tok/s (est.) | ~60 tok/s (est.) |

Numbers are highly memory-bandwidth-bound. Smaller models give near-linear speedups.

## Roadmap

- [ ] Get Vulkan build working end-to-end (shader-gen is slow in parallel make)
- [ ] Tool/function calling passthrough — agent tool schemas in the prompt
- [ ] Measured benchmark on real SD680 hardware
- [ ] Dynamic model reload (`POST /v1/models/reload`) without daemon restart
- [ ] Optional per-request sampling override (temperature, top_p)

## License

AGPL-3.0-or-later (matches the parent repo). llama.cpp, cpp-httplib, and nlohmann/json are all MIT/Apache licensed.
