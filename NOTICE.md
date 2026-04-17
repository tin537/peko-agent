# NOTICE

Peko Agent — Copyright 2024-2026 Tanuphat Chainaloedwong / ftmstars

This project is released under the **GNU Affero General Public License v3.0 or later**
(see `LICENSE` and `LICENSE-AGPL`).

The distributed source tree and compiled binaries include third-party software.
The copyright and license of each component is preserved in the source headers
and, where applicable, reproduced in full in `third_party/LICENSES/`.

---

## Third-Party Software Included or Linked

The following components are **vendored** in this repository:

### cpp-httplib

- Source: https://github.com/yhirose/cpp-httplib
- Version: **v0.18.7**
- License: **MIT**
- Copyright: © 2025 Yuji Hirose
- Location in repo: `crates/peko-llm-daemon/third_party/httplib.h`
- Full license text: `third_party/LICENSES/cpp-httplib.LICENSE`
- Use: HTTP/1.1 server bound to Unix Domain Socket in `peko-llm-daemon`

### nlohmann/json

- Source: https://github.com/nlohmann/json
- Version: **v3.11.3**
- License: **MIT**
- Copyright: © 2013-2023 Niels Lohmann <https://nlohmann.me>
- Location in repo: `crates/peko-llm-daemon/third_party/json.hpp`
- Full license text: `third_party/LICENSES/nlohmann-json.LICENSE`
- Use: JSON request/response parsing in `peko-llm-daemon`

---

## Third-Party Software Fetched at Build Time

The following components are **not** stored in this repository but are
downloaded by CMake FetchContent during the build and statically linked
into the distributed binary. Their license texts are reproduced in
`third_party/LICENSES/` for convenience and to satisfy redistribution
requirements.

### llama.cpp

- Source: https://github.com/ggml-org/llama.cpp
- Pinned commit: **`268d61e178f5de67b0f5eaed3bc84b1e6daccf96`** (see `crates/peko-llm-daemon/CMakeLists.txt`)
- License: **MIT**
- Copyright: © 2023-2026 Georgi Gerganov and the llama.cpp / ggml-org contributors
- Full license text: `third_party/LICENSES/llama.cpp.LICENSE`
- Use: LLM inference engine (CPU + optional Vulkan GPU backend) in `peko-llm-daemon`

### ggml

- Source: bundled with llama.cpp (above), originally https://github.com/ggerganov/ggml
- License: **MIT** (same as llama.cpp)
- Copyright: © 2023-2026 Georgi Gerganov
- Full license text: same as llama.cpp (`third_party/LICENSES/llama.cpp.LICENSE`)
- Use: tensor backend used by llama.cpp for quantized inference

---

## Rust Dependencies

Standard Cargo dependencies (tokio, serde, reqwest, axum, candle, etc.) are
fetched from crates.io and have their own licenses (almost all MIT / Apache-2.0).
See each crate's LICENSE on crates.io for details.

A full Rust dependency license inventory can be generated with:

```bash
cargo install cargo-about
cargo about generate about.hbs
```

---

## License Compatibility

AGPL-3.0-or-later is compatible with the MIT / Apache-2.0 licenses of the
third-party code above. MIT / Apache-licensed code can be incorporated into
an AGPL work, and the combined work is governed by AGPL. The original MIT
notices are preserved as required by their terms.

---

## AGPL §13 — Network Use

When Peko Agent is deployed as a network-facing service (e.g. via the web UI
on port 8080, or as a bot), operators **must** offer the Corresponding Source
Code to every user interacting with it remotely. The web UI includes a link
to the source repository for this purpose. If you deploy a modified fork, you
must serve the source of your fork.

See `LICENSE-AGPL` §13 for the precise terms.

---

## Commercial Licensing

A separate commercial license is *planned* but not yet finalized. Until the
commercial terms are drafted and reviewed by counsel, this project is
effectively distributed under AGPL-3.0-or-later only. See `LICENSE-COMMERCIAL`
for the current status and contact information for inquiries.
