# ReAct: Reasoning + Acting

> The foundational paradigm for all agent architectures.

---

## Citation

Yao, Zhao, Yu, Du, Shafran, Narasimhan, and Cao. **"ReAct: Synergizing Reasoning and Acting in Language Models."** ICLR 2023.

## Core Idea

Interleave **reasoning traces** (natural language thinking) with **grounded actions** (tool calls):

```
Thought: I need to find the current screen state before I can navigate.
Action: screenshot {}
Observation: [PNG image of home screen]
Thought: I can see the Settings icon at approximately (540, 1800). I'll tap it.
Action: touch {"action": "tap", "x": 540, "y": 1800}
Observation: Tapped at (540, 1800)
Thought: Now I should take another screenshot to verify I'm in Settings.
Action: screenshot {}
...
```

## Why Interleaving Matters

| Approach | Reasoning | Actions | Performance |
|---|---|---|---|
| Chain-of-thought only | Yes | No | Good at reasoning, can't act |
| Action-only | No | Yes | Acts blindly, makes errors |
| **ReAct** | **Yes** | **Yes** | **Best of both** |

ReAct demonstrated significant improvements on HotpotQA and WebShop benchmarks.

## How Peko Agent Implements ReAct

The [[../implementation/ReAct-Loop|agent loop]] in [[../implementation/peko-core|peko-core]] is a direct implementation:

```
Observe: Tool results + screenshot data → conversation context
Think:   LLM generates reasoning (text content in response)
Act:     LLM generates tool calls (tool_use blocks in response)
```

The loop continues until the LLM produces a text-only response (no tool calls), indicating the task is complete.

## Extended Thinking

Modern LLMs (Claude with extended thinking, o1-style models) add an internal reasoning step:

```
[Internal thinking — not shown to user]
The user wants to navigate to WiFi settings. I can see the Settings
icon at the bottom of the home screen. After tapping it, I'll need
to scroll down to find the Network section...

[Visible response]
I'll navigate to the WiFi settings for you.

[Tool call]
touch {"action": "tap", "x": 540, "y": 1800}
```

Peko Agent supports this via `ThinkingDelta` in [[../implementation/SSE-Streaming|StreamEvent]] — thinking content is logged but not added to the visible conversation.

## Toolformer Connection

**Toolformer** (Schick et al., NeurIPS 2023) showed that tool use can be **embedded within generation** — the model decides when to call a tool as part of its output. This is exactly how modern API tool calling works:

```json
{
  "content": [
    {"type": "text", "text": "Let me take a screenshot."},
    {"type": "tool_use", "name": "screenshot", "input": {}}
  ]
}
```

The tool call is part of the generation, not a separate post-processing step.

## Agent Architecture Patterns (Masterman et al., 2024)

The survey identifies four hallmarks of effective agents:

| Hallmark | Peko Agent implementation |
|---|---|
| Well-defined system prompt | SystemPrompt builder (SOUL + MEMORY + schemas) |
| Planning-execution-evaluation phases | ReAct loop with observation feedback |
| Dynamic team structures | Subagent delegation capability |
| Intelligent message filtering | [[../implementation/Context-Compression\|ContextCompressor]] |

## Related

- [[../implementation/ReAct-Loop]] — Peko Agent's implementation
- [[Agent-Benchmarks]] — How ReAct agents are evaluated
- [[Mobile-Agents]] — ReAct applied to mobile devices
- [[Computer-Use-Agents]] — ReAct applied to desktop

---

#research #react #foundation #paradigm
