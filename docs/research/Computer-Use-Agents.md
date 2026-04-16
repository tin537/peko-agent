# Computer Use Agents

> Desktop and visual grounding agents — the broader paradigm.

---

## Claude Computer Use (Anthropic, October 2024)

The closest conceptual relative to Peko Agent, but for desktop.

**How it works**:
- Claude interprets screenshots of the desktop
- Generates mouse movements, clicks, and keyboard input
- Client-side tool architecture executes actions
- Iterates: screenshot → reason → act → screenshot

**Key difference from Peko Agent**: Claude CU runs as a client-side integration (Python/JS toolkit on the user's machine). Peko Agent runs as the system itself.

**Evaluation**: Tested on OSWorld benchmark.

## OSWorld (NeurIPS 2024)

The primary benchmark for computer use agents.

| Detail | Value |
|---|---|
| Tasks | 369 |
| Platforms | Ubuntu, Windows, macOS |
| Human performance | 72.36% |
| Best automated agent | ~38% |
| Gap | ~34 percentage points |

Shows that computer use agents have significant room for improvement. Peko Agent's direct hardware access could reduce latency and improve reliability compared to framework-mediated approaches.

## CogAgent (CVPR 2024 Highlight)

**Key idea**: Purpose-built VLM for GUI navigation.

| Detail | Value |
|---|---|
| Model size | 18B parameters |
| Resolution | 1120x1120 (dual encoder: low-res + high-res) |
| Benchmarks | SOTA on Mind2Web (PC) and AITW (Android) |

**Relevance**: CogAgent shows that specialized visual models significantly outperform general-purpose VLMs for GUI understanding. Peko Agent could use CogAgent as the vision backbone for local inference. See [[../implementation/LLM-Providers|PekoLocalProvider]].

## Set-of-Mark (SoM) Prompting (Microsoft Research, 2023)

**Key idea**: Overlay alphanumeric markers on UI elements so the LLM can reference them by ID.

```
Before SoM: "Click the submit button" (LLM must locate it)
After SoM:  Screenshot has markers [A] [B] [C] on elements
            "Click [B]" (LLM just picks the marker)
```

Uses SAM/SEEM for segmentation, then overlays numbered markers.

**Relevance**: Could be implemented as a pre-processing step in [[../implementation/peko-tools-android|ScreenshotTool]] — annotate the screenshot before sending to the LLM. This reduces the visual grounding burden.

## SeeAct (ICML 2024)

**Full title**: "GPT-4V(ision) is a Generalist Web Agent, if Grounded"

| Detail | Value |
|---|---|
| With oracle grounding | 51.1% task completion |
| With automated grounding | ~26-31% |
| Grounding gap | 20-25% |

**Key finding**: Visual grounding (mapping language to UI elements) is the primary bottleneck, not reasoning. This informs Peko Agent's design — investing in better screenshot preprocessing and coordinate extraction is higher-value than better reasoning prompts.

## Mind2Web (NeurIPS 2023 Spotlight)

Web navigation benchmark:
- 2,350 tasks from 137 real-world websites
- Tests generalization across websites
- Includes HTML and screenshot modalities

## WebVoyager (ACL 2024)

End-to-end multimodal web agent:
- 59.1% task success on 15 popular websites
- Pure vision approach (no HTML parsing)
- Validates screenshot-only perception

## Screen2Words (UIST 2021)

Automatic mobile UI summarization:
- 112,000 language summaries across 22,000 unique Android screens
- Multimodal input (screenshot + view hierarchy + text + metadata)
- Shows that multimodal > single-modality for screen understanding

**Relevance**: When available, combining [[../implementation/peko-tools-android|UiDumpTool]] (XML) with screenshots is better than either alone.

## Visual Grounding: The Core Challenge

Every computer use agent faces the same bottleneck:

```
Screenshot → "Where exactly is the 'Settings' icon?" → (x, y) coordinates
```

Current approaches:
1. **XML/HTML parsing** — precise but requires framework access
2. **OCR + heuristics** — works for text, fails for icons
3. **Vision LLM directly** — improving rapidly but still imprecise
4. **Set-of-Mark** — annotate first, then reference by ID
5. **Specialized VLMs** — CogAgent, purpose-trained for GUI

Peko Agent supports multiple strategies:
- Direct screenshot + vision LLM (always available)
- UiDumpTool XML (when framework is running)
- SoM preprocessing (future enhancement)

## Related

- [[Mobile-Agents]] — Android-specific agents
- [[Agent-Benchmarks]] — Evaluation frameworks
- [[../01-Vision]] — How Peko Agent differs from all of these
- [[../implementation/peko-tools-android]] — Our perception and action tools
- [[../roadmap/Possibilities]] — Future visual grounding enhancements

---

#research #computer-use #vision #grounding
