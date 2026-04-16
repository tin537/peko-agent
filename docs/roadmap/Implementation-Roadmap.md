# Implementation Roadmap

> Phased plan from paper to running binary.

---

## Principles

1. **Desktop-first development** — get the agent loop working on macOS/Linux before touching Android
2. **Incremental hardware integration** — add one kernel interface at a time
3. **Test at every layer** — mock boundaries, verify contracts
4. **Ship the simplest useful thing first** — a working ReAct loop with text tools, then add vision

## Phase Overview

```
Phase 1 ─── Foundation ────────── peko-core + config + types
  │         (desktop)              Agent loop running with mock tools
  ▼
Phase 2 ─── Transport ────────── peko-transport + SSE + providers
  │         (desktop)              Real LLM calls working
  ▼
Phase 3 ─── Hardware ─────────── peko-hal
  │         (Android device)       evdev, framebuffer, serial wrappers
  ▼
Phase 4 ─── Tools ────────────── peko-tools-android
  │         (Android device)       Screenshot, touch, SMS, etc.
  ▼
Phase 5 ─── Integration ──────── peko-agent binary
  │         (Android device)       Socket, signals, full binary
  ▼
Phase 6 ─── Deploy ───────────── init.rc, SELinux, device boot
            (rooted device)        Agent boots from init
```

## Timeline Estimate

| Phase | Effort | Depends on |
|---|---|---|
| [[Phase-1-Foundation]] | ~1-2 weeks | Nothing |
| [[Phase-2-Transport]] | ~1-2 weeks | Phase 1 |
| [[Phase-3-Hardware]] | ~2-3 weeks | Rooted Android device |
| [[Phase-4-Tools]] | ~2-3 weeks | Phase 1 + 3 |
| [[Phase-5-Integration]] | ~1-2 weeks | Phase 1-4 |
| [[Phase-6-Android-Deploy]] | ~1-2 weeks | Phase 5 + device |

**Total: ~8-14 weeks** for a motivated developer.

Phases 1-2 can be done entirely on desktop. Phase 3+ requires a rooted Android device.

## Critical Path

```
Phase 1 → Phase 2 → Phase 5
                ↗
Phase 3 → Phase 4
```

Phase 3 (hardware) and Phase 2 (transport) can run in parallel if you have both a developer and a device available.

## Definition of Done (per phase)

| Phase | Done when... |
|---|---|
| 1 | Agent completes a multi-step task using mock tools on desktop |
| 2 | Agent calls real Anthropic/OpenRouter API, parses streamed responses correctly |
| 3 | Can read framebuffer, inject touch events, send AT commands on real device |
| 4 | Agent takes screenshot, taps screen, sends SMS on real device |
| 5 | Single binary with control socket runs as standalone daemon |
| 6 | Binary boots from init.rc, survives reboot, accessible via socket |

## Milestones

### M1: "Hello World Agent" (End of Phase 2)
Agent running on desktop, calling Anthropic API, executing shell commands and file operations. Proof of concept for the ReAct loop.

### M2: "See and Touch" (End of Phase 4)
Agent running on Android device, taking screenshots, tapping the screen based on LLM vision. First real device interaction.

### M3: "Agent-as-OS" (End of Phase 6)
Agent boots from init.rc, operates independently, accepts tasks via control socket. The full vision realized.

## Related

- [[Phase-1-Foundation]] through [[Phase-6-Android-Deploy]] — detailed phase plans
- [[Testing-Strategy]] — Testing approach at each phase
- [[Device-Requirements]] — What hardware you need
- [[Challenges-And-Risks]] — What could go wrong

---

#roadmap #plan #overview
