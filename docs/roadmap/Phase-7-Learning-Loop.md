# Phase 7: The Learning Loop

> Memory, skills, and user modeling — closing the gap with original Peko Agent.

---

## Why This Matters

Without the learning loop, the agent is a **stateless tool executor**. It does
exactly what you ask, forgets immediately, and starts from zero every time.

With the learning loop, the agent becomes a **growing intelligence**:
- Remembers what worked (memory)
- Builds reusable procedures (skills)
- Understands who you are (user model)
- Gets better the longer it runs

This is Peko Agent's defining innovation and our biggest gap.

---

## Phase 7a: Memory System (~1-2 weeks)

### What it does

The agent remembers important facts across sessions. Before each task, it
searches past memories for relevant context and injects them into the prompt.
It also periodically "nudges" itself: "Is there anything from this conversation
I should remember?"

### Architecture

```
┌─────────────────────────────────────────┐
│ Memory System                           │
│                                         │
│  ┌──────────┐  ┌──────────────────────┐ │
│  │ memories  │  │ SQLite + FTS5        │ │
│  │ table     │──│ Full-text search     │ │
│  │           │  │ across all memories  │ │
│  └──────────┘  └──────────────────────┘ │
│       │                                  │
│       ▼                                  │
│  Before each LLM call:                   │
│  1. Search memories for task keywords    │
│  2. Inject top-K relevant memories       │
│     into system prompt                   │
│  3. Add nudge: "save important facts"    │
│                                          │
│  New tool: "memory"                      │
│  - save(key, content)                    │
│  - search(query) → relevant memories     │
│  - list() → all memories                 │
│  - delete(key)                           │
└──────────────────────────────────────────┘
```

### Database Schema

```sql
CREATE TABLE memories (
    id TEXT PRIMARY KEY,
    key TEXT NOT NULL,           -- Short label
    content TEXT NOT NULL,       -- The memory itself
    category TEXT DEFAULT 'fact', -- fact, preference, procedure, observation
    importance REAL DEFAULT 0.5, -- 0.0 to 1.0
    source_session TEXT,         -- Which session created it
    created_at TEXT NOT NULL,
    accessed_at TEXT,            -- Last time injected into prompt
    access_count INTEGER DEFAULT 0
);

CREATE VIRTUAL TABLE memories_fts USING fts5(
    key, content, category,
    content='memories',
    content_rowid='rowid'
);
```

### Memory Nudge

Every N iterations (configurable, default 5), append to the system prompt:

```
[Memory Nudge] Consider if anything from this conversation should be
remembered for future sessions. If so, use the memory tool to save it.
Categories: fact, preference, procedure, observation.
```

The agent decides what to remember — no forced extraction.

### Memory Injection

Before calling the LLM, search memories:

```rust
let query = extract_keywords(&user_input);
let relevant = memory_store.search(&query, limit=5)?;
let memory_text = format_memories(&relevant);
system_prompt += &format!("\n## Relevant Memories\n{}", memory_text);
```

### Tasks

- [ ] Add `memories` table + FTS5 to SessionStore (or new MemoryStore)
- [ ] Create `MemoryTool` implementing Tool trait (save/search/list/delete)
- [ ] Add memory search → system prompt injection in AgentRuntime
- [ ] Add periodic nudge (every N iterations)
- [ ] Add `/api/memories` endpoint for web UI
- [ ] Add Memory tab/section in web UI
- [ ] Config: `memory_enabled`, `nudge_interval`, `max_memories_injected`

---

## Phase 7b: Skills System (~1-2 weeks)

### What it does

When the agent successfully completes a multi-step task, it can save the
procedure as a "skill" — a reusable template it can invoke next time without
re-reasoning the whole thing.

### Architecture

```
┌─────────────────────────────────────────┐
│ Skills System                           │
│                                         │
│  /data/peko/skills/                   │
│  ├── open_settings.md                   │
│  ├── send_sms.md                        │
│  ├── take_screenshot_and_describe.md    │
│  └── navigate_to_wifi.md               │
│                                         │
│  Skill file format:                     │
│  ---                                    │
│  name: navigate_to_wifi                 │
│  description: Open WiFi settings        │
│  created: 2026-04-16                    │
│  success_count: 3                       │
│  ---                                    │
│  ## Steps                               │
│  1. Press HOME key                      │
│  2. Take screenshot                     │
│  3. Find "Settings" icon                │
│  4. Tap on Settings                     │
│  5. Scroll down to find "Network"       │
│  6. Tap "Network & internet"            │
│  7. Tap "WiFi"                          │
│                                         │
│  New tool: "skills"                     │
│  - create(name, description, steps)     │
│  - list() → available skills            │
│  - use(name) → inject steps into prompt │
│  - improve(name, new_steps)             │
│  - delete(name)                         │
└──────────────────────────────────────────┘
```

### Skill Discovery

Before each task, the agent sees available skills:

```
## Available Skills
- navigate_to_wifi: Open WiFi settings (used 3 times, 100% success)
- send_sms: Send an SMS message (used 5 times, 80% success)

If a skill matches your task, follow its steps. If you discover a
better approach, use the skills tool to improve it.
```

### Self-Improvement

When a skill's steps fail, the agent adapts and can update the skill:

```
Agent: "The WiFi settings moved in the new Android update. Let me
update the skill with the correct navigation path."
→ skills.improve("navigate_to_wifi", new_steps)
```

### Tasks

- [ ] Define skill file format (YAML frontmatter + markdown steps)
- [ ] Create SkillStore (load from /data/peko/skills/, CRUD)
- [ ] Create `SkillsTool` implementing Tool trait
- [ ] Inject available skills into system prompt
- [ ] Track success/failure per skill (update frontmatter)
- [ ] Add `/api/skills` endpoint for web UI
- [ ] Add Skills section in Device tab or new tab

---

## Phase 7c: User Model (~1 week)

### What it does

The agent builds a persistent model of the user — preferences, expertise,
communication style, common tasks. This makes interactions more natural
and efficient over time.

### Architecture

```
┌─────────────────────────────────────────┐
│ User Model                              │
│                                         │
│  /data/peko/user_model.json           │
│  {                                      │
│    "name": "...",                        │
│    "language": "en",                     │
│    "expertise": "developer",            │
│    "preferences": {                     │
│      "verbose_responses": false,        │
│      "confirm_dangerous": true,         │
│      "preferred_apps": ["Chrome", ...], │
│    },                                   │
│    "patterns": {                        │
│      "common_tasks": ["send sms", ...], │
│      "active_hours": "09:00-22:00",     │
│    },                                   │
│    "observations": [                    │
│      "prefers short responses",         │
│      "often asks about weather",        │
│    ]                                    │
│  }                                      │
│                                         │
│  Updated via memory nudge:              │
│  "What have I learned about the user    │
│   from this conversation?"              │
└──────────────────────────────────────────┘
```

### Injection

User model summary injected into system prompt:

```
## About the User
Name: [name]. Developer-level expertise. Prefers concise responses.
Common tasks: SMS, WiFi settings, app management.
Observation: Usually asks follow-up questions, prefers step-by-step.
```

### Tasks

- [ ] Define UserModel struct (JSON serializable)
- [ ] Load/save from /data/peko/user_model.json
- [ ] Add user model observations via memory nudge
- [ ] Inject user model summary into system prompt
- [ ] Add user profile section in web UI Config tab
- [ ] Config: `user_model_enabled`

---

## Phase 7d: SOUL.md Personality (~2-3 days)

### What it does

Replace the hardcoded system prompt with a customizable SOUL.md file that
defines the agent's personality, instructions, and behavior.

### Implementation

```
/data/peko/SOUL.md — loaded at startup, editable via web UI

# Peko Agent

You are Peko, an autonomous AI agent running directly on an
Android device at the OS level...

## Personality
- Direct and concise
- Technical but accessible
- Always verify actions with screenshots

## Rules
- Never send SMS without confirmation
- Always take a screenshot before and after UI actions
- Save skills for tasks you complete successfully
```

### Tasks

- [ ] Load SOUL.md from data_dir if exists, else use default
- [ ] Replace hardcoded `DEFAULT_SOUL` in prompt.rs
- [ ] Add SOUL.md editor in web UI Config tab
- [ ] Save SOUL.md to disk when edited

---

#roadmap #phase-7 #learning-loop #memory #skills
