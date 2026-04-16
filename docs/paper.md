# Peko Agent: an agent-as-OS architecture in Rust for frameworkless Android

**Peko Agent rewrites the NousResearch Peko Agent as a single Rust binary that boots directly from Android's `init.rc`, bypassing Zygote, ART, and the entire Android framework to run an LLM-powered autonomous agent with raw kernel-level device control.** This design eliminates the ~800 MB memory overhead of the Android runtime, gives the agent sub-millisecond access to hardware through Linux kernel interfaces, and removes the permission sandbox that constrains conventional Android applications. The architecture treats the LLM agent not as an app inside an OS, but as the OS-level orchestrator itself—an "Agent-as-OS" paradigm where the reasoning loop sits at PID-1-child privilege, directly mediating between cloud LLM inference and bare metal. This paper provides both the academic grounding and the complete implementation specification required to build the system.

---

## Section 1: Academic background and related work

### 1.1 LLM-based autonomous agents and the ReAct paradigm

The theoretical foundation for Peko Agent's agent loop derives from **ReAct** (Yao, Zhao, Yu, Du, Shafran, Narasimhan, and Cao; ICLR 2023), which established the interleaved reasoning-and-acting paradigm where an LLM alternates between generating natural-language thought traces and executing grounded actions against external environments. ReAct demonstrated that interleaving reasoning with action significantly outperforms both chain-of-thought (reasoning-only) and action-only baselines on tasks like HotpotQA and WebShop, and its observe→think→act loop has become the de facto architecture for all production agent systems, including the Peko Agent.

The capacity for LLMs to autonomously select and invoke tools was formalized by **Toolformer** (Schick, Dwivedi-Yu, Dessì, Raileanu, Lomeli, Hambro, Zettlemoyer, Cancedda, and Scialom; NeurIPS 2023), which showed that language models can learn in a self-supervised manner when to call APIs, what arguments to pass, and how to incorporate results into subsequent generation. Toolformer's insight that tool use can be embedded within the generation process—rather than layered on top—directly informs Peko Agent's design, where tool invocations are parsed from the model's output stream in real time via SSE delta processing.

Systematic evaluation of LLM agent capabilities was established by **AgentBench** (Liu, Yu, Zhang, Xu, Lei, Lai, Gu, Ding, Men, Yang, et al.; ICLR 2024), which benchmarks agents across eight environments including OS interaction, database queries, and web navigation. AgentBench revealed a **significant performance gap** between top commercial LLMs (GPT-4 class) and open-source models on agentic tasks, identifying poor long-term reasoning, decision-making, and instruction following as the primary obstacles. This finding motivates Peko Agent's provider-agnostic transport layer, which allows hot-switching between cloud models (Anthropic Claude, OpenRouter) and local models (peko 3/4) to balance capability against latency and cost.

The survey "The Landscape of Emerging AI Agent Architectures for Reasoning, Planning, and Tool Calling" (Masterman, Besen, Sawtell, and Chao; arXiv 2024) synthesizes agent implementation patterns across single-agent and multi-agent systems, identifying four hallmarks of effective agent architectures: well-defined system prompts, dedicated reasoning/planning-execution-evaluation phases, dynamic team structures, and intelligent message filtering. Peko Agent adopts all four through its prompt builder, iterative tool loop, subagent delegation capability, and context compressor.

### 1.2 Mobile device agents and Android automation

The application of LLM agents to smartphone control has produced a rapid succession of systems. **AppAgent** (Zhang, Yang, Liu, Han, Chen, Huang, Fu, and Yu; CHI 2025, originally arXiv December 2023) from Tencent introduced a multimodal agent framework powered by GPT-4V that operates smartphone applications through simplified tap-and-swipe actions. AppAgent operates in two phases—exploration (learning app functionality through autonomous interaction, stored as knowledge documents) and deployment (executing user tasks)—and was evaluated on 50 tasks across 10 applications. Its key limitation, which Peko Agent addresses, is that it requires the Android accessibility service and framework to extract UI element coordinates from XML view hierarchies.

**Mobile-Agent** (Wang, Xu, Ye, Yan, Shen, Zhang, Huang, and Sang; ICLR 2024) from Alibaba and Beijing Jiaotong University took a vision-centric approach, using OCR and CLIP-based visual perception to identify and locate UI elements from screenshots alone, without relying on XML files or system metadata. Mobile-Agent demonstrated that purely visual understanding can achieve competitive task completion on its Mobile-Eval benchmark, though it requires external perception tools because GPT-4V alone cannot reliably output precise pixel coordinates. The successor **Mobile-Agent-v2** (Wang et al.; NeurIPS 2024) introduced a multi-agent architecture with dedicated planning, decision, and reflection agents. Peko Agent draws from this vision-first philosophy, using `screencap` for screenshot capture and sending raw pixel data to vision-capable LLMs for UI understanding.

**AutoDroid** (Wen, Li, Liu, Zhao, Yu, Li, Jiang, Liu, Zhang, and Liu; ACM MobiCom 2024) combines LLM commonsense knowledge with app-specific domain knowledge through automated dynamic analysis. Its key innovations include functionality-aware UI representation, exploration-based memory injection, and multi-granularity query optimization, achieving **90.9% action accuracy and 71.3% task completion** on 158 tasks—outperforming GPT-4-powered baselines by over 36%. AutoDroid's predecessor **DroidBot-GPT** (Li et al.; arXiv 2023) was the first Android app automator guided by an LLM, translating GUI elements and action spaces into prompts for ChatGPT.

**AndroidWorld** (Rawles, Clinckemaillie, Chang, Waltz, Lau, Fair, Li, Bishop, Li, Campbell-Ajala, Toyama, Berry, Tyamagundlu, Lillicrap, and Riva; arXiv 2024) from Google Research provides a fully functional Android environment with reward signals for **116 programmatic tasks across 20 real-world apps**, with dynamic task construction enabling millions of unique variations. Built on **AndroidEnv** (Toyama, Hamel, Gergely, Comanici, Glaese, Ahmed, Jackson, Mourad, and Precup; arXiv 2021), Google DeepMind's RL platform for Android, AndroidWorld established that even the best agents (M3A, at 30.6% success) fall far short of human performance (80%). The **Android in the Wild (AITW)** dataset (Rawles et al.; NeurIPS 2023) provides the field's largest device-control dataset with **715,000 episodes** spanning 30,000 unique instructions, serving as the standard training and evaluation corpus.

**DigiRL** (Bai, Zhou, Cemri, Pan, Suhr, Levine, and Kumar; NeurIPS 2024) demonstrated that autonomous reinforcement learning can dramatically improve device control agents, achieving **67.2% success rate on AITW**—a 49.5% absolute improvement over supervised fine-tuning (17.7%) and far surpassing AppAgent with GPT-4V (8.3%). DigiRL's two-stage training (offline RL initialization followed by offline-to-online RL) addresses real-world stochasticity that static demonstrations cannot capture.

The software engineering perspective on Android automation is represented by **AdbGPT** (Feng and Chen; ICSE 2024), formally titled "Prompting Is All You Need: Automated Android Bug Replay with Large Language Models," which uses few-shot learning and chain-of-thought prompting to automatically reproduce 81.3% of Android bugs from natural language bug reports. AdbGPT's approach of encoding GUI screens into HTML-like text for LLM consumption informs Peko Agent's UI representation strategy. Earlier work on learning human interaction patterns includes **Humanoid** (Li, Yang, Guo, and Chen; ASE 2019), a deep learning approach to generating human-like test inputs by learning from interaction traces.

### 1.3 Computer use agents and visual grounding

The broader paradigm of LLM-controlled computer interaction has been advanced significantly by **Anthropic's Claude Computer Use** (public beta, October 2024), which enables Claude to interact with desktop environments by interpreting screenshots, moving cursors, clicking buttons, and typing text through a client-side tool architecture. Claude Computer Use is evaluated on the **OSWorld** benchmark (Xie, Zhang, Chen, Li, Zhao, Cao, Hua, Cheng, Shin, Lei, Liu, Xu, Zhou, Savarese, Xiong, Zhong, and Yu; NeurIPS 2024), the first scalable real computer environment for multimodal agents, supporting task setup and execution-based evaluation across Ubuntu, Windows, and macOS with **369 tasks**. Human performance on OSWorld is 72.36%, while the best automated agents achieve approximately 38%.

Visual grounding—the ability to map natural-language references to specific UI elements in screenshots—is a critical bottleneck identified across all GUI agent work. **CogAgent** (Hong, Wang, Lv, Xu, Yu, Ji, Wang, Wang, Dong, Ding, and Tang; CVPR 2024 Highlight) is an 18-billion-parameter visual language model with dual low-resolution and high-resolution image encoders supporting **1120×1120 input resolution**, achieving state-of-the-art performance on both PC (Mind2Web) and Android (AITW) GUI navigation benchmarks. **Set-of-Mark (SoM) Prompting** (Yang, Zhang, Li, Zou, Li, and Gao; arXiv 2023, Microsoft Research) provides an alternative approach by overlaying alphanumeric markers on image regions segmented by SAM/SEEM, enabling GPT-4V to reference specific UI elements by identifier—this technique is widely adopted in subsequent agent architectures.

**SeeAct** (Zheng, Gou, Kil, Sun, and Su; ICML 2024), formally "GPT-4V(ision) is a Generalist Web Agent, if Grounded," demonstrated that GPT-4V can complete 51.1% of live website tasks with oracle grounding but identified a **20–25% gap** between automated and oracle grounding as the primary bottleneck. For web navigation specifically, **Mind2Web** (Deng, Gu, Zheng, Chen, Stevens, Wang, Sun, and Su; NeurIPS 2023 Spotlight) provides 2,350 tasks from 137 real-world websites, and **WebVoyager** (He, Yao, Ma, Yu, Dai, Zhang, Lan, and Yu; ACL 2024) achieves 59.1% task success on 15 popular websites using an end-to-end multimodal approach.

**Screen2Words** (Wang, Li, Zhou, Chen, Grossman, and Li; UIST 2021) established the task of automatic mobile UI summarization, training a Transformer encoder-decoder on **112,000 language summaries across 22,000 unique Android UI screens** from the RICO dataset, demonstrating that multimodal input (screenshot + view hierarchy + text + app metadata) significantly outperforms single-modality approaches for screen understanding.

### 1.4 OS-level agents and system-level AI

**OS-Copilot** (Wu, Han, Ding, Weng, Liu, Yao, Yu, and Kong; ICLR 2024), formally "Towards Generalist Computer Agents with Self-Improvement," introduced a framework for building agents that interface with comprehensive OS elements including web, code terminals, files, multimedia, and third-party applications. OS-Copilot's agent FRIDAY uses a DAG-based planner, a configurator with working memory and tool retrieval, and an actor with self-criticism, outperforming previous methods by **35% on the GAIA benchmark** and demonstrating self-directed learning on desktop applications. While OS-Copilot operates within a standard OS environment, Peko Agent pushes this further by making the agent the first non-init process, with direct kernel interface access rather than going through OS abstractions.

### 1.5 Rust for systems programming and Android

Google's formal adoption of Rust for AOSP, announced in April 2021, provides direct precedent for Peko Agent's implementation language choice. As of Android 13, approximately **21% of new native code** in AOSP is written in Rust, totaling ~1.5 million lines. Major Rust components in production Android include Keystore2, the UWB stack, DNS-over-HTTP3, and portions of the Bluetooth stack. Google reported that memory safety bugs dropped from **223 in 2019 to 85 in 2022**, directly attributed to the shift toward memory-safe languages. Rust is compiled via rustc directly (not Cargo) for integration into Android's Soong build system, with Rust/C++ interop supported via the CXX crate and AIDL-generated Rust bindings.

The formal foundations for Rust's safety guarantees were established by **RustBelt** (Jung et al.; POPL 2018), the first machine-checked safety proof for a realistic subset of Rust, verifying key standard library types including Arc and Mutex. The broader systems programming case was made by "System Programming in Rust: Beyond Safety" (Balasubramanian et al.; HotOS 2017), which demonstrated that Rust's linear type system enables capabilities beyond memory safety, including zero-copy software fault isolation with only **90 cycles overhead per protected method call**. "Safe Systems Programming in Rust: The Promise and the Challenge" (Jung et al.; Communications of the ACM, 2021) extended this with the RustBelt verification framework. For embedded contexts specifically, "Rust for Embedded Systems: Current State and Open Problems" (Sharma et al.; arXiv 2023) evaluated FFI binding generation and RTOS integration, finding Rust's type system a superset of C's for interoperability purposes.

Android security architecture research relevant to Peko Agent's SELinux integration includes "Understanding and Defending the Binder Attack Surface in Android" (Feng and Shin; ACSAC 2016), which analyzed over 100 Binder vulnerabilities, and "Securing Android-Powered Mobile Devices Using SELinux" (Shabtai et al.; IEEE Security & Privacy, 2009), which laid the groundwork for SEAndroid. The Rust-for-Linux project, merged in Linux 6.1+, provides kernel-level Rust support with a specific focus on the Binder driver as a target for Rust reimplementation.

---

## Section 2: System architecture design

### 2a. Boot sequence: starting as a PID 1 child via Android init

Peko Agent boots as a first-class native daemon launched by Android's init process (PID 1) before the Zygote process and the entire Android framework. The boot sequence proceeds as follows.

**Stage 1 — Kernel to init.** The bootloader loads the Linux kernel, which initializes interrupt controllers, memory protections, caches, and scheduling, sets up virtual memory, mounts the root filesystem, and launches `/init` as PID 1. Android's init process executes in three internal stages: first-stage init mounts `/dev`, `/proc`, `/sys`, and early-mount partitions (`system`, `vendor`); SELinux setup compiles and loads the mandatory access control policy; second-stage init parses `init.rc` scripts and enters its main event loop.

**Stage 2 — init.rc service definition.** Peko Agent is defined as a `class core` service in a dedicated `.rc` file placed in `/system/etc/init/` or `/vendor/etc/init/`:

```
service peko-agent /system/bin/peko-agent \
    --config /data/peko/config.toml
    class core
    user root
    group root input graphics audio radio inet net_raw
    capabilities NET_RAW NET_ADMIN SYS_PTRACE
    seclabel u:r:peko_agent:s0
    socket peko stream 0660 root root
    writepid /dev/cpuset/foreground/tasks
    oneshot
    disabled
```

The `class core` designation ensures Peko Agent starts during the `on boot` trigger, which fires **before** `class_start main` (which launches Zygote, SurfaceFlinger, and other framework services). The service is initially `disabled` and triggered explicitly by a property trigger or `on` action to allow hardware initialization to complete:

```
on property:sys.peko.start=1
    start peko-agent
```

Alternatively, for a fully frameworkless device where Zygote is never needed, the `late-init` trigger can be modified to skip `class_start main` entirely:

```
on late-init
    trigger early-fs
    trigger fs
    trigger post-fs
    trigger post-fs-data
    trigger peko-boot

on peko-boot
    class_start core
    # Deliberately omit: class_start main
    start peko-agent
```

**Stage 3 — SELinux policy.** A custom SELinux domain must be created for the Peko Agent process. The policy files include:

Type enforcement (`peko_agent.te`):
```
type peko_agent, domain;
type peko_agent_exec, exec_type, file_type, system_file_type;
init_daemon_domain(peko_agent)

# Kernel device access
allow peko_agent input_device:chr_file { open read write ioctl };
allow peko_agent gpu_device:chr_file rw_file_perms;
allow peko_agent graphics_device:chr_file rw_file_perms;
allow peko_agent tty_device:chr_file rw_file_perms;
allow peko_agent self:capability { net_raw net_admin sys_ptrace };

# Network access for LLM API calls
allow peko_agent self:tcp_socket create_stream_socket_perms;
allow peko_agent self:udp_socket create_socket_perms;
allow peko_agent port:tcp_socket name_connect;

# Filesystem access
allow peko_agent peko_data_file:dir create_dir_perms;
allow peko_agent peko_data_file:file create_file_perms;

# Binder IPC (optional, for HAL access)
allow peko_agent binder_device:chr_file rw_file_perms;
binder_use(peko_agent)
```

File contexts (`file_contexts`):
```
/system/bin/peko-agent  u:object_r:peko_agent_exec:s0
/data/peko(/.*)?         u:object_r:peko_data_file:s0
```

**Stage 4 — Peko Agent initialization.** When started by init, the Peko Agent binary performs the following startup sequence:

1. **Parse configuration** from `/data/peko/config.toml` (API keys, model selection, tool configuration, iteration limits).
2. **Initialize tokio runtime** — creates the async executor with a multi-threaded scheduler sized to available cores.
3. **Initialize SQLite** — opens or creates the session database at `/data/peko/state.db` with FTS5 extension for full-text search.
4. **Register tools** — the `ToolRegistry` scans and registers all compiled-in tool implementations (screenshot, touch injection, SMS, call, file operations, etc.).
5. **Probe hardware** — enumerate available `/dev/input/event*` devices to identify the touchscreen, check for modem at `/dev/ttyACM*`, verify framebuffer/DRM availability.
6. **Open control socket** — listen on the Unix domain socket `/dev/socket/peko` for external control commands (start task, query status, interrupt).
7. **Enter agent loop** — begin the main ReAct loop, either waiting for commands on the control socket or executing a pre-configured startup task.

**Dependency graph.** Peko Agent depends only on the Linux kernel, `libc`, and the following kernel subsystems: networking stack (TCP/IP for LLM API calls), input subsystem (`/dev/input/*` for touch injection), framebuffer or DRM (`/dev/graphics/fb0` or `/dev/dri/*` for screenshots), and optionally the serial/USB subsystem (`/dev/ttyACM*` for modem AT commands). It does **not** depend on: Zygote, ART/Dalvik, SystemServer, SurfaceFlinger (can bypass via direct framebuffer reads), Binder IPC framework (can optionally use raw binder ioctls for HAL communication), or any Java/Kotlin code.

### 2b. Rust crate architecture

The Peko Agent workspace is organized as a Cargo workspace with well-defined crate boundaries enforcing separation of concerns through Rust's trait system. Each crate compiles to a distinct compilation unit, enabling parallel builds and clear dependency management.

#### `peko-core` — Agent runtime and orchestration engine

This is the central crate containing the agent's brain. It has zero platform-specific dependencies, enabling testing on desktop Linux.

**`AgentRuntime`** is the top-level orchestrator that owns the agent loop. Its public interface:

```rust
pub struct AgentRuntime {
    config: AgentConfig,
    tools: Arc<ToolRegistry>,
    budget: IterationBudget,
    compressor: ContextCompressor,
    session: SessionStore,
    provider: Box<dyn LlmProvider>,
    conversation: Vec<Message>,
    system_prompt: SystemPrompt,
}

impl AgentRuntime {
    pub async fn new(config: AgentConfig, tools: Arc<ToolRegistry>,
                     provider: Box<dyn LlmProvider>) -> Result<Self>;
    pub async fn run_task(&mut self, user_input: &str) -> Result<AgentResponse>;
    pub async fn run_loop(&mut self) -> Result<()>; // Continuous mode
    pub fn interrupt(&self); // Signal loop to stop
}
```

The `run_task` method implements the core ReAct loop:

1. Append user message to `conversation`.
2. Call `system_prompt.build()` to assemble the full system prompt (SOUL + MEMORY + tool schemas + context files).
3. Check context budget via `compressor.check_and_compress(&mut conversation)`.
4. Call `provider.stream_completion(system_prompt, conversation, tools.schemas())`.
5. Process the SSE stream: accumulate text deltas, detect tool calls.
6. If tool calls are present: dispatch each through `tools.execute(name, args)`, append tool results to conversation, decrement `budget`, loop to step 3.
7. If final text response with no tool calls: return `AgentResponse`, persist to `session`.

**`ToolRegistry`** manages tool registration, schema generation, and dispatch through a trait-based design:

```rust
pub trait Tool: Send + Sync + 'static {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value; // JSON Schema
    fn is_available(&self) -> bool { true }
    fn is_dangerous(&self) -> bool { false }
    fn execute(&self, args: serde_json::Value)
        -> Pin<Box<dyn Future<Output = Result<ToolResult>> + Send + '_>>;
}

pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn register(&mut self, tool: impl Tool);
    pub fn schemas(&self) -> Vec<serde_json::Value>; // For LLM prompt
    pub async fn execute(&self, name: &str, args: serde_json::Value)
        -> Result<ToolResult>;
    pub fn available_tools(&self) -> Vec<&str>;
}
```

Every tool implements the `Tool` trait. The registry collects JSON schemas from all registered tools to inject into the system prompt, and dispatches calls by name at runtime. Tools that are `is_dangerous()` require explicit confirmation through the control socket before execution.

**`IterationBudget`** provides thread-safe iteration limiting using atomic operations, enabling the agent loop to be interrupted from any thread:

```rust
pub struct IterationBudget {
    remaining: Arc<AtomicUsize>,
    max: usize,
    interrupted: Arc<AtomicBool>,
}

impl IterationBudget {
    pub fn new(max_iterations: usize) -> Self;
    pub fn decrement(&self) -> Result<usize>; // Err if exhausted
    pub fn remaining(&self) -> usize;
    pub fn interrupt(&self); // Sets interrupted flag
    pub fn is_interrupted(&self) -> bool;
    pub fn reset(&self);
}
```

The `Arc<AtomicUsize>` design allows the budget to be shared between the agent loop thread and the control socket listener thread, enabling external interruption without locks. The default budget is 50 iterations per task, configurable via `config.toml`.

**`ContextCompressor`** manages the conversation context window to prevent exceeding the model's token limit. It implements a head-tail compaction strategy:

```rust
pub struct ContextCompressor {
    max_context_tokens: usize,
    history_share: f32, // Fraction of context budget for history (default 0.7)
    token_counter: Box<dyn TokenCounter>,
}

impl ContextCompressor {
    pub fn check_and_compress(&self, conversation: &mut Vec<Message>)
        -> Result<CompressionResult>;
}
```

The compression algorithm preserves the first message (system prompt) and the last N messages (recent context), summarizing middle turns using either a local heuristic (extract tool names and final results only) or an auxiliary LLM call. When the conversation exceeds `max_context_tokens × history_share`, the compressor replaces middle turns with a single summary message. This is critical for long-running tasks on mobile where memory pressure is a concern.

Additional `peko-core` modules include:

- **`SystemPrompt`** — Assembles the system prompt from template components: SOUL.md (agent personality and instructions), MEMORY.md (persistent facts, capped at ~2,200 chars), tool schemas, and dynamic context files. The system prompt is stable within a single task execution (never changes mid-conversation), following the Peko Agent's design principle.
- **`SessionStore`** — SQLite persistence layer using `rusqlite` with FTS5 for full-text search across past sessions. Stores complete conversation histories, tool call logs, and task outcomes. Schema includes `sessions(id, started_at, task, status)` and `messages(id, session_id, role, content, tool_name, tool_args, created_at)` tables.
- **`Message` types** — Strongly-typed message representation: `Message::System(String)`, `Message::User(String)`, `Message::Assistant { text: Option<String>, tool_calls: Vec<ToolCall> }`, `Message::ToolResult { tool_use_id: String, name: String, content: String, is_error: bool }`.

#### `peko-transport` — Async HTTP client and LLM API streaming

This crate handles all network communication with LLM providers, abstracting over provider-specific API differences behind a unified trait.

**Provider trait:**

```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn stream_completion(
        &self,
        system_prompt: &str,
        messages: &[Message],
        tools: &[serde_json::Value],
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>>;

    fn model_name(&self) -> &str;
    fn max_context_tokens(&self) -> usize;
}
```

**`StreamEvent` enum** unifies all streaming events across providers:

```rust
pub enum StreamEvent {
    MessageStart { id: String, input_tokens: usize },
    ContentBlockStart { index: usize, block_type: ContentBlockType },
    TextDelta(String),
    ToolUseStart { id: String, name: String },
    ToolInputDelta(String), // Partial JSON for tool arguments
    ThinkingDelta(String),
    ContentBlockStop { index: usize },
    MessageDelta { output_tokens: usize, stop_reason: StopReason },
    MessageStop,
    Ping,
}
```

**Anthropic provider implementation.** The `AnthropicProvider` handles streaming against `POST https://api.anthropic.com/v1/messages` with `stream: true`. It processes the SSE event sequence: `message_start` → `content_block_start` → `content_block_delta` (multiple) → `content_block_stop` → `message_delta` → `message_stop`. Tool use arrives as `content_block_start` with `type: "tool_use"` followed by `input_json_delta` events that must be accumulated and parsed as complete JSON when `content_block_stop` fires. The implementation handles Anthropic-specific features including prompt caching (via `cache_control` breakpoints on system prompt blocks) and extended thinking (`thinking` content blocks with `signature_delta`).

```rust
pub struct AnthropicProvider {
    client: reqwest::Client,
    api_key: String,
    model: String,
    max_tokens: usize,
    base_url: String, // default: https://api.anthropic.com
}
```

HTTP headers required: `anthropic-version: 2023-06-01`, `content-type: application/json`, `x-api-key: {api_key}`. The request body follows the Anthropic Messages API schema with `messages`, `system`, `tools`, `max_tokens`, and `stream: true`.

**OpenRouter/OpenAI-compatible provider implementation.** The `OpenAICompatProvider` handles the OpenAI chat completions format used by OpenRouter (`https://openrouter.ai/api/v1/chat/completions`), Nous Portal, and any OpenAI-compatible endpoint. SSE streaming uses `data: {json}\n\n` format with `choices[0].delta` containing incremental content and tool calls. Tool calls arrive as `delta.tool_calls[i].function.arguments` string fragments that must be accumulated per tool call index.

```rust
pub struct OpenAICompatProvider {
    client: reqwest::Client,
    api_key: String,
    model: String,
    base_url: String,
    max_tokens: usize,
    extra_headers: HeaderMap, // Provider-specific headers
}
```

**peko native model provider.** For local peko models (peko 3, peko 4), the `pekoLocalProvider` generates prompts in ChatML format (`<|im_start|>` / `<|im_end|>`) and parses tool calls from `<tool_call>` XML tags in the model output. Tool definitions are injected in `<tools>` tags in the system prompt with JSON schemas. Tool results are formatted as `<tool_response>` blocks with the `tool` role. This provider connects to a local inference server (vLLM, llama.cpp, or similar) via the OpenAI-compatible API.

**SSE parser.** A dedicated `SseParser` struct handles the low-level parsing of Server-Sent Events from the HTTP response body stream:

```rust
pub struct SseParser {
    buffer: String,
}

impl SseParser {
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<SseEvent>;
}

pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
    pub id: Option<String>,
}
```

The parser handles multi-line data fields, event type discrimination, reconnection IDs, and the `[DONE]` sentinel used by OpenAI-compatible APIs. It operates on raw byte chunks from `reqwest`'s streaming response, making it zero-copy where possible.

**HTTP client configuration.** The `reqwest::Client` is configured with: connection pooling (keep-alive), TLS via `rustls` (avoiding OpenSSL dependency), configurable timeouts (30s connect, 300s read for long generations), automatic retry with exponential backoff on 5xx errors, and HTTP/2 for multiplexing. The client is built once at startup and shared via `Arc` across all provider instances.

**Provider failover.** The `ProviderChain` wraps multiple `LlmProvider` implementations and attempts them in priority order, falling back to the next provider on connection errors or rate limits (HTTP 429). This mirrors the Peko Agent's credential pool and automatic fallback system.

#### Additional workspace crates

**`peko-tools-android`** — Android-specific tool implementations:

- **`ScreenshotTool`** — Captures the current screen by reading `/dev/graphics/fb0` (legacy) or using DRM/KMS ioctls on `/dev/dri/*` (modern devices), encoding to PNG via the `image` crate. Falls back to executing the `screencap` binary if SurfaceFlinger is running. Returns base64-encoded PNG for inclusion in multimodal LLM messages.
- **`TouchTool`** — Injects touch events by writing `input_event` structs directly to the touchscreen's `/dev/input/eventN` device. Implements tap (down + up), long press (down + delay + up), and swipe (down + series of move events + up) gestures. Each event requires proper `EV_ABS` codes (`ABS_MT_POSITION_X`, `ABS_MT_POSITION_Y`, `ABS_MT_TRACKING_ID`, `ABS_MT_PRESSURE`) followed by `EV_SYN` / `SYN_REPORT`. Device enumeration at startup via `ioctl(EVIOCGNAME)` identifies the correct event node.
- **`KeyEventTool`** — Sends key events (HOME, BACK, POWER, VOLUME_UP/DOWN, ENTER) via `/dev/input/eventN` for the keyboard/button device. Uses `EV_KEY` event type with appropriate keycodes.
- **`TextInputTool`** — Types text into the focused input field. For the frameworkless case, this injects individual key events for ASCII characters or uses the clipboard + paste mechanism.
- **`SmsTool`** — Sends SMS messages via AT commands to the modem. Opens the serial device (typically `/dev/ttyACM0` or a device identified via `/sys/class/tty/`), configures with `termios` settings (115200 baud, 8N1), and sends `AT+CMGF=1` (text mode), `AT+CMGS="<number>"` followed by the message body and Ctrl-Z (0x1A).
- **`CallTool`** — Initiates and manages phone calls via AT commands: `ATD<number>;` to dial, `ATH` to hang up, `ATA` to answer incoming calls. Monitors call state via unsolicited result codes (`RING`, `NO CARRIER`, `BUSY`).
- **`UiDumpTool`** — If the Android framework is partially running, executes `uiautomator dump` to capture the UI hierarchy as XML. In fully frameworkless mode, relies on screenshot + vision LLM for UI understanding.
- **`NotificationTool`** — Reads notifications via `/sys/class/leds/` for LED state or `dumpsys notification` if the framework is available.
- **`FileSystemTool`** — Standard file operations (read, write, list, search, delete) using `std::fs` with path sandboxing to prevent accidental system damage.
- **`ShellTool`** — Executes arbitrary shell commands via `tokio::process::Command`, capturing stdout/stderr with timeout enforcement. Equivalent to Peko Agent's `terminal_tool.py` but targeting Android's shell environment.

**`peko-hal`** — Hardware abstraction for direct kernel interface access:

- **`InputDevice`** — Enumerates and manages `/dev/input/event*` devices, providing typed wrappers around `input_event` structs and `ioctl` calls. Uses the `nix` crate for safe ioctl wrappers.
- **`Framebuffer`** — Reads screen content from `/dev/graphics/fb0` via `mmap`, extracting pixel data as RGBA buffers. Handles variable screen info (`FBIOGET_VSCREENINFO`) and fixed screen info (`FBIOGET_FSCREENINFO`) to determine resolution, stride, and pixel format.
- **`DrmDisplay`** — Modern alternative to framebuffer, using DRM/KMS ioctls on `/dev/dri/card0` for screen capture on devices that have deprecated `fb0`.
- **`SerialModem`** — Manages the serial connection to the cellular modem, handling `termios` configuration, AT command send/receive with timeout, and unsolicited response code parsing.
- **`UInputDevice`** — Creates virtual input devices via `/dev/uinput` for injecting events without requiring identification of the physical touchscreen device node.

**`peko-config`** — Configuration parsing and management:

- Deserializes `config.toml` via `serde` + `toml` crate.
- Configuration struct covers: LLM provider settings (API keys, model names, base URLs, failover order), tool enable/disable flags, iteration budget, context window limits, data directory paths, logging level, SELinux context, hardware device paths (overrides for auto-detection).
- Supports environment variable overrides (`peko_API_KEY`, `peko_MODEL`, etc.) and Android system property reads (`getprop`).

**`peko-agent` (binary crate)** — The final binary that ties everything together:

- `main.rs` initializes the tokio runtime, reads configuration, creates the `ToolRegistry` with Android tools, instantiates the `LlmProvider` chain, constructs the `AgentRuntime`, opens the Unix control socket, and enters the main event loop.
- The control socket accepts JSON-RPC commands: `{"method": "run_task", "params": {"input": "..."}}`, `{"method": "status"}`, `{"method": "interrupt"}`, `{"method": "shutdown"}`.
- Signal handling: catches `SIGTERM` (from init on shutdown) and `SIGHUP` (configuration reload) via `tokio::signal`.
- Logging via `tracing` crate with output to both logcat (via Android's `__android_log_write`) and a file log at `/data/peko/peko.log`.

**Build and cross-compilation.** The workspace is cross-compiled for `aarch64-linux-android` using the Android NDK toolchain. The `.cargo/config.toml` specifies:

```toml
[target.aarch64-linux-android]
linker = "aarch64-linux-android31-clang"
rustflags = ["-C", "link-arg=-landroid", "-C", "link-arg=-llog"]
```

The final binary is a statically-linked (where possible) ELF executable placed at `/system/bin/peko-agent`, with a size target under 15 MB. Dependencies are minimized: `reqwest` with `rustls-tls` (no OpenSSL), `rusqlite` with bundled SQLite, `serde`/`serde_json`, `tokio`, `image` (PNG encoding only), `nix` (ioctl wrappers), `tracing`, and `toml`.

### Workspace dependency graph

```
peko-agent (binary)
├── peko-core (agent loop, tool registry, context, session)
│   ├── peko-transport (HTTP, SSE, LLM providers)
│   └── peko-config (configuration)
├── peko-tools-android (Android tool implementations)
│   └── peko-hal (kernel device interfaces)
├── peko-hal (input, framebuffer, modem, uinput)
└── peko-config
```

All inter-crate communication uses trait objects (`Box<dyn Tool>`, `Box<dyn LlmProvider>`) and message types from `peko-core`, ensuring that the core agent logic is fully decoupled from both the transport layer and the platform-specific tool implementations. This enables desktop testing by swapping `peko-tools-android` with a `peko-tools-desktop` crate that wraps standard Linux tools, and swapping `peko-hal` with mock implementations.

## Conclusion

Peko Agent represents a fundamentally new position in the design space of LLM agents. Where existing mobile agents like AppAgent, Mobile-Agent, and AutoDroid operate as applications within the Android framework—constrained by permission sandboxes, dependent on accessibility services, and competing with the OS for resources—Peko Agent operates *as* the system layer itself. The architecture exploits a specific insight: Android's Linux kernel provides all the hardware interfaces an agent needs (input injection via `evdev`, screen capture via framebuffer/DRM, telephony via serial AT commands, networking via sockets), and the Android framework's ~800 MB overhead exists primarily to support multi-app GUI paradigms that an autonomous agent does not need.

The Rust implementation provides three critical properties for this use case. First, memory safety without garbage collection ensures the agent daemon can run indefinitely without memory leaks or GC pauses—essential for a PID-1-child process. Second, the `async`/`await` model via tokio enables efficient concurrent handling of SSE streaming, tool execution, and control socket communication on mobile hardware with limited cores. Third, Rust's trait system enables the clean separation between the provider-agnostic agent core and platform-specific tool implementations, making the codebase testable on desktop while deploying on Android.

The novel contribution is not any single component—ReAct loops, SSE parsing, and Android input injection are all well-understood—but their composition into a single binary that replaces the traditional OS application stack. This "Agent-as-OS" architecture opens research directions in agent-hardware co-design, on-device tool learning, and minimal-footprint autonomous systems that merit further exploration.
