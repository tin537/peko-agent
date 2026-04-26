# Peko Agent v0.4.1 — "Background tasks that survive"

Released 2026-04-26.

## What this release means

v0.4.0 shipped fire-and-forget background jobs (`bg fire ...`) but they
were in-memory only: every agent restart silently dropped whatever was
mid-flight, and there was no upper bound on how much LLM budget a
runaway tool loop could burn.

v0.4.1 closes both gaps.

- **Phase 21**: SQLite-backed catalog at `<data_dir>/bg.db`, daily
  token + wall-clock + iteration caps, `bg stats` action that lets
  the agent inspect its own usage patterns.
- **Phase 22**: per-iteration mid-run checkpoints. A bg job that was
  `Running` when the agent died gets resumed from its last
  conversation snapshot. Stale or checkpoint-less Running rows get
  auto-failed with a clear reason — no silent orphans.
- **Telegram tools widened**: `touch`, `key_event`, `text_input` are
  now opt-in addable to `[telegram].allowed_tools`; UI driving over
  Telegram now actually works for owners of the bot.
- **Lockscreen prompt nudge**: the system prompt explicitly tells the
  agent how to detect the keyguard and forbids "bypass / circumvent /
  hack" framing that trips cloud LLM safety filters.
- **Log hygiene**: ANSI colors only when stdout is a real terminal;
  `peko.log` is greppable now.

## Phase 21 — Persistence + Budget

### What's persistent

| Table | Purpose |
|---|---|
| `bg_jobs` | Catalog of every fire ever run (id, task, status, timing, result, error, session, iterations, tokens_used) plus reserved `checkpoint_blob` / `checkpoint_at` / `schema_version` columns |
| `bg_stats` | Per-day counters keyed by `(date, metric)` |

### Default caps (`[bg]` section, `config.toml`)

```toml
[bg]
max_tokens_per_day = 200000     # daily LLM token budget; rejects new fires when exceeded
max_wall_clock_secs = 600       # hard kill any bg job that runs >10 min
max_iterations = 30             # ReAct loop cap per job
max_concurrent = 8              # max simultaneous Running jobs
```

### Tracked metrics

`fired`, `completed`, `failed`, `cancelled`, `timeout`,
`budget_rejected`, `tokens_used`, `iterations`, `resumed`, `orphaned`.

The agent can ask `bg stats { days: 7 }` to see its own usage trend
and self-improve.

## Phase 22 — Mid-run resume

After every ReAct iteration the bg worker writes a MessagePack-encoded
`Checkpoint { task, iterations, tokens_so_far, messages }` into the
job's `checkpoint_blob` column. On agent startup, `BgStore::pending_resumable(1h)`:

1. Returns `(BgJob, Checkpoint)` for every `Running` row whose
   checkpoint is fresher than 1 hour. Each gets re-spawned via
   `spawn_worker` with `with_resume_state(...)` pre-seeded.
2. Auto-marks Failed (with `ORPHANED` metric bumped) any `Running`
   row that has no checkpoint or one older than the window — so a
   crashed agent never leaves the catalog poisoned.

## Bug fixes / hardening

- Cloud safety filter trip: the previous SOUL prompt let the LLM
  reach for "bypass" framing on lockscreen tasks, which `mimo-v2-omni`
  refuses. Prompt now mandates calling `unlock_device` directly with
  no euphemisms.
- `Message` + `ImageData` now derive `Serialize` / `Deserialize` so
  they can ride in checkpoint blobs.
- ANSI escape sequences no longer pollute `peko.log` when the agent
  is run under nohup / init / Magisk.

## On-device verified

| Test | Result |
|---|---|
| 11 background-store unit tests, incl. checkpoint roundtrip + 1-hour resume window | ✅ |
| Synthetic Running-row-without-checkpoint on real device, agent restart auto-orphans | ✅ |
| `bg.db` schema deployed at `/data/peko/bg.db` with `checkpoint_blob` + `bg_stats` tables | ✅ |
| Telegram unlock flow with `touch` re-enabled, agent unlocks in 3 iterations | ✅ |

## Migration from v0.4.0

The bg.db file is created on first start; no migration step needed.
The `[bg]` config section is optional — defaults are applied when
absent. `[telegram].allowed_tools` is a user-edited whitelist; if
you want UI driving over Telegram, append `"touch"`, `"key_event"`,
`"text_input"`.

## What's next

Phase 23 candidates queued:

- Per-job Telegram completion notification (close the polling gap).
- Auto-prune `bg_jobs` via the existing memory gardener.
- Per-token usage tracking from provider response headers (replace
  the heuristic `estimate_tokens` once mimo-v2-omni surfaces them).
