# Mobile Agents

> Survey of LLM-powered mobile device control agents.

---

## Landscape

```
Timeline:
2023 ──────────────────────────────────────── 2025
  │                                            │
  DroidBot-GPT   AppAgent   Mobile-Agent-v2    │
  (first LLM     (Tencent)  (multi-agent)      │
   Android)       │          │                  │
                  Mobile-    AutoDroid           │
                  Agent      (90.9% accuracy)    │
                  (vision)                       │
                             DigiRL             │
                             (67.2% AITW)       │
                                                │
                  AdbGPT     Peko Agent ◄───┘
                  (bug        (Agent-as-OS)
                   replay)
```

## AppAgent (Tencent, CHI 2025)

**Key idea**: GPT-4V-powered agent with simplified tap/swipe actions.

| Aspect | Detail |
|---|---|
| Model | GPT-4V |
| Perception | Screenshots + XML view hierarchy |
| Actions | Tap, swipe, type |
| Learning | Two-phase: explore (learn app) → deploy (execute task) |
| Evaluation | 50 tasks across 10 apps |

**Limitation**: Requires Android accessibility service and framework for XML extraction.

**Relevance to Peko Agent**: Demonstrates the viability of LLM-driven mobile control. Peko Agent removes the framework dependency.

## Mobile-Agent (Alibaba, ICLR 2024)

**Key idea**: Vision-only — no XML, just screenshots with OCR + CLIP.

| Aspect | Detail |
|---|---|
| Model | GPT-4V |
| Perception | Screenshots + OCR + CLIP for element detection |
| Actions | Tap, swipe, type, navigate |
| Evaluation | Custom Mobile-Eval benchmark |

**Mobile-Agent-v2** (NeurIPS 2024): Multi-agent — separate planning, decision, and reflection agents.

**Relevance to Peko Agent**: Validates the vision-first approach. Since Peko Agent uses [[../knowledge/Screen-Capture|direct screen capture]] + vision LLM, it follows this philosophy. No XML hierarchy needed.

## AutoDroid (ACM MobiCom 2024)

**Key idea**: Combines LLM commonsense with app-specific domain knowledge via dynamic analysis.

| Aspect | Detail |
|---|---|
| Accuracy | **90.9% action accuracy**, 71.3% task completion |
| Innovation | Functionality-aware UI representation, exploration-based memory |
| Evaluation | 158 tasks, outperforms GPT-4 baselines by 36% |

Predecessor **DroidBot-GPT** (2023) was the first LLM Android automator.

**Relevance to Peko Agent**: AutoDroid's accuracy shows what's achievable. Peko Agent aims for similar or better accuracy with less overhead.

## DigiRL (NeurIPS 2024)

**Key idea**: Reinforcement learning for device control — not just imitation learning.

| Aspect | Detail |
|---|---|
| AITW success rate | **67.2%** (vs 17.7% supervised, 8.3% AppAgent+GPT-4V) |
| Method | Offline RL → offline-to-online RL (two-stage) |
| Key insight | Real-world stochasticity requires online learning, not just demonstrations |

**Relevance to Peko Agent**: DigiRL's RL approach could be integrated as a future enhancement — the agent learning from its own device interactions. See [[../roadmap/Possibilities]].

## AdbGPT (ICSE 2024)

**Key idea**: Automated Android bug replay from natural language reports.

| Aspect | Detail |
|---|---|
| Success rate | 81.3% bug reproduction |
| Method | Few-shot + chain-of-thought prompting |
| UI representation | HTML-like text encoding of GUI screens |

**Relevance to Peko Agent**: AdbGPT's UI-as-HTML encoding informs Peko Agent's strategy for when [[../implementation/peko-tools-android|UiDumpTool]] is available.

## Evaluation Environments

### AndroidWorld (Google Research, 2024)

- **116 programmatic tasks** across 20 real-world apps
- Dynamic task construction → millions of unique variations
- Best agent (M3A): **30.6% success** vs human **80%**
- Built on AndroidEnv (DeepMind)

### Android in the Wild (AITW, NeurIPS 2023)

- **715,000 episodes**, 30,000 unique instructions
- Largest device-control dataset
- Standard training/evaluation corpus

### Humanoid (ASE 2019)

- Deep learning approach to generating human-like test inputs
- Learns interaction patterns from traces

## Common Architecture Patterns

All mobile agents share these components:

```
1. Perception ──► screenshot + optional XML/OCR
2. Reasoning  ──► LLM generates plan/action
3. Action     ──► tap/swipe/type via accessibility/ADB
4. Feedback   ──► new screenshot → loop
```

Peko Agent follows the same pattern but replaces the perception and action layers with direct kernel access. See [[../01-Vision]].

## Related

- [[Computer-Use-Agents]] — Desktop equivalents
- [[Agent-Benchmarks]] — Evaluation details
- [[../01-Vision]] — How Peko Agent differs
- [[../implementation/peko-tools-android]] — Our action implementation

---

#research #mobile-agents #android
