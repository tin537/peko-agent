# Contributing to Peko Agent

Thanks for thinking about helping out. Peko Agent is an opinionated, hands-on
project — a Rust binary that thinks it's a phone OS — so contributions that
respect the architecture and the AGPL licence are welcome.

This guide covers three tracks:

1. **Filing bugs** — things that crashed, froze, or did the wrong thing.
2. **Feature proposals** — new tools, new providers, new surface area.
3. **Pull requests** — code changes, doc fixes, examples.

## Before you start

- Read [`docs/00-Home.md`](docs/00-Home.md) — the architecture + implementation
  map. Most design decisions are justified in there rather than in ticket
  history.
- Skim [`docs/architecture/Architecture-Overview.md`](docs/architecture/Architecture-Overview.md)
  and [`docs/implementation/Tool-System.md`](docs/implementation/Tool-System.md)
  so PRs don't re-invent layers.
- Check whether your idea is already covered under
  [`docs/roadmap/`](docs/roadmap/). A lot of "why isn't X here?" questions are
  answered by a phase that hasn't shipped yet.
- Read [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md) once.

## Filing a bug

Open a GitHub issue with:

- **Environment** — OS / ROM / root (Magisk? what version?), device model,
  `adb shell getprop ro.build.version.release`.
- **Reproduction** — the smallest sequence of steps that triggers it.
- **Expected vs actual** — two lines each is usually enough.
- **Logs** — `adb shell cat /data/peko/peko.log | tail -200` (redact API
  keys if you pasted one in `config.toml` and it leaked into a stack trace).
- **Config** — the relevant `[section]` from `config.toml` with secrets
  redacted.

Crashes in `peko-llm-daemon` should include `adb shell cat /data/peko/daemon.log`.

## Proposing a feature

Open an issue with the `proposal` label (or just `[proposal]` in the title).
Explain:

- **The problem.** What are you trying to do that peko can't do today?
- **The minimum surface.** One new tool? A new provider? A web UI tab?
- **What you'd accept as an answer.** "I want a toggle in the Life tab"
  reaches agreement faster than "peko should be smarter".

Features that extend `peko-tools-android` should assume they might run on a
dozing phone under SELinux — fragile paths are rejected on sight. Features
that cost money (cloud LLM, SMS, paid APIs) need a rate-limit + audit log
by default.

## Pull requests

### Setup

```bash
git clone <your fork>
cd peko_agent
rustup target add aarch64-linux-android      # only if touching device code
cargo build                                   # host smoke-build
cargo test                                    # workspace tests (~103 green)
```

For Android-side work you also need:

- Android Studio JBR (JDK 17+) — the Kotlin overlay + sms-shim modules expect
  it at `/Applications/Android Studio.app/Contents/jbr/Contents/Home` on
  macOS (override via `JAVA_HOME`).
- NDK 27+ at `$ANDROID_HOME/ndk/*` for the cross-compile.

### What makes a PR reviewable

- **Scope.** One logical change per PR. A bug fix PR shouldn't smuggle in a
  refactor; a refactor PR shouldn't reformat unrelated files.
- **Tests.** If you touched a crate that has tests
  (`peko-core`, `peko-transport`, `peko-config`), extend them. If your
  change is a new tool, the integration test layout in
  `crates/peko-tools-android/src/tests.rs` is your model.
- **No backwards-compat cruft.** We don't ship feature flags or compat
  shims for unreleased changes. If a breaking refactor is warranted,
  just break it.
- **Errors that mean something.** `anyhow::bail!(...)` with a message a
  user can act on; not `"something went wrong"`.
- **Comments only for the WHY.** Name your variables well and let the
  code explain *what* it does; reserve comments for the non-obvious
  reasoning. We aggressively strip boilerplate doc blocks during review.
- **AGPL header not required** on new source files — the LICENSE at the
  repo root applies to every file in the workspace.

### Rust style

- `cargo fmt` before you push.
- `cargo clippy --workspace -- -D warnings` should stay clean. A handful
  of existing dead-code / unused-mut warnings are grandfathered; don't
  add new ones.
- Tests go in `#[cfg(test)] mod tests` at the bottom of the module, not
  in a separate `tests/` file unless they exercise cross-crate integration.

### Kotlin style

- The Android modules use standard Android Kotlin conventions — 4 spaces,
  `private` by default, receivers / services small enough to read top-to-
  bottom.
- Don't reach for DI frameworks or ViewModels in the overlay / shim.
  These are priv-apps built to be understood by someone reading one file;
  keep them that way.

### Commit hygiene

We don't require conventional commits, but:

- **One commit per logical change** (squash-merge happens upstream).
- **Imperative subject line.** "Add X" not "Added X".
- **Explain the WHY in the body** when the diff doesn't.
- No `Co-Authored-By: Claude` lines. If you authored the change, sign your
  own name.

### Submitting

Open a PR against `main`. Expect a review within a few days; if it's been a
week with no response, poke the issue. The project is run by a small number of
maintainers, not a team — patience helps.

## Licence reminder

By contributing code to Peko Agent, you agree that your contribution is
licensed under **AGPL-3.0-or-later**, matching the rest of the project, and
that you have the right to offer it on those terms. There is no CLA and no
alternative licence.

If your employer has claims on code you write on their time, sort that out
before you file a PR — we can't take contributions that would put the project
at risk.
