# Agent Benchmarks

> How LLM agents are evaluated — and where the gaps are.

---

## Benchmark Landscape

```
General-purpose            Mobile-specific           Desktop
┌──────────────┐          ┌──────────────┐          ┌──────────────┐
│  AgentBench  │          │ AndroidWorld │          │   OSWorld    │
│  (8 envs)    │          │ (116 tasks)  │          │  (369 tasks) │
└──────────────┘          └──────────────┘          └──────────────┘
                          ┌──────────────┐          ┌──────────────┐
                          │    AITW      │          │   Mind2Web   │
                          │ (715K eps)   │          │ (2,350 tasks)│
                          └──────────────┘          └──────────────┘
                          ┌──────────────┐
                          │  Mobile-Eval │
                          │  (custom)    │
                          └──────────────┘
```

## AgentBench (ICLR 2024)

**8 environments**: OS interaction, database queries, web navigation, knowledge graph, digital card game, lateral thinking puzzles, household tasks, web shopping.

**Key finding**: Significant gap between top commercial LLMs (GPT-4 class) and open-source models on agentic tasks. Primary obstacles:
- Poor long-term reasoning
- Weak decision-making
- Inconsistent instruction following

**Relevance to Peko Agent**: Motivates the [[../implementation/LLM-Providers|provider-agnostic design]] — ability to hot-swap between capable cloud models and lighter local models.

## AndroidWorld (Google Research, 2024)

| Detail | Value |
|---|---|
| Tasks | 116 programmatic tasks |
| Apps | 20 real-world apps |
| Task variations | Millions (dynamic construction) |
| Best agent (M3A) | **30.6% success** |
| Human performance | **80%** |
| Gap | **~50 percentage points** |

Built on **AndroidEnv** (Google DeepMind) — an RL platform for Android.

**Key insight**: Even the best agents fail 70% of the time on realistic Android tasks. There is massive room for improvement.

## Android in the Wild (AITW, NeurIPS 2023)

The field's largest device-control dataset:

| Detail | Value |
|---|---|
| Episodes | **715,000** |
| Unique instructions | 30,000 |
| Devices | Multiple real Android devices |
| Task types | Web, apps, system settings |

Standard training/evaluation corpus. [[Mobile-Agents|DigiRL]] achieved 67.2% on AITW — the current SOTA.

## OSWorld (NeurIPS 2024)

Primary benchmark for [[Computer-Use-Agents|computer use agents]]:

| Detail | Value |
|---|---|
| Tasks | 369 |
| Platforms | Ubuntu, Windows, macOS |
| Human performance | **72.36%** |
| Best automated | **~38%** |
| Gap | **~34 percentage points** |

Evaluation uses execution-based checking (not just action matching) — did the task actually complete?

## Mind2Web (NeurIPS 2023 Spotlight)

Web navigation benchmark:
- 2,350 tasks from 137 real-world websites
- Tests generalization across unseen websites
- Three test sets: cross-task, cross-website, cross-domain

## What the Numbers Tell Us

### Current State of the Art

| Benchmark | Best Agent | Success Rate | Human Baseline |
|---|---|---|---|
| AndroidWorld | M3A | 30.6% | 80% |
| AITW | DigiRL | 67.2% | ~90% |
| OSWorld | Best automated | ~38% | 72.4% |

### The Grounding Gap

[[Computer-Use-Agents|SeeAct]] found a 20-25% gap between oracle grounding and automated grounding. This means:
- Agents can reason about tasks correctly ~50% of the time
- But they fail to map that reasoning to precise UI coordinates ~25% of the time
- **Improving visual grounding yields more gains than improving reasoning**

### Implications for Peko Agent

1. **Direct hardware access won't fix reasoning** — the LLM still needs to understand the task
2. **But it removes action-execution failures** — no accessibility API bugs, no framework delays
3. **Latency reduction helps** — faster screenshot → think → act cycles mean more iterations in the same time budget
4. **The benchmark gap is an opportunity** — current agents are far from human performance

## How Peko Agent Would Be Evaluated

For rigorous evaluation, Peko Agent should be tested on:

1. **AndroidWorld subset** — tasks that don't require framework APIs
2. **Custom benchmark** — agent-as-OS specific tasks (SMS, calls, navigation without framework)
3. **Latency measurements** — action execution speed vs framework-based agents
4. **Resource usage** — memory, CPU, battery consumption

See [[../roadmap/Testing-Strategy]] for the practical testing plan.

## Related

- [[Mobile-Agents]] — Agent implementations
- [[Computer-Use-Agents]] — Desktop agents and grounding research
- [[ReAct-Paper]] — The foundational paradigm
- [[../roadmap/Testing-Strategy]] — Peko Agent testing approach

---

#research #benchmarks #evaluation
