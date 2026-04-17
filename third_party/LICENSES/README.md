# Third-Party License Texts

This directory contains the full license texts of third-party software that
is either vendored in this repository or fetched at build time and statically
linked into distributed binaries.

See [`../../NOTICE.md`](../../NOTICE.md) for the consolidated attribution list.

| File | Component | License | Usage |
|------|-----------|---------|-------|
| `cpp-httplib.LICENSE` | cpp-httplib v0.18.7 | MIT | vendored in `crates/peko-llm-daemon/third_party/httplib.h` |
| `nlohmann-json.LICENSE` | nlohmann/json v3.11.3 | MIT | vendored in `crates/peko-llm-daemon/third_party/json.hpp` |
| `llama.cpp.LICENSE` | llama.cpp + ggml | MIT | fetched at build time, statically linked into `peko-llm-daemon` |

If you redistribute a built binary of `peko-agent` or `peko-llm-daemon`,
you must include these license notices (e.g., bundled as a resource,
printed via `--licenses`, or distributed alongside the binary).
