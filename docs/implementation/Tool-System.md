# Tool System

> Trait-based tool architecture for extensible agent capabilities.

---

## Design Philosophy

The tool system follows a simple principle: **the agent's capabilities are defined by which tools are registered.** Adding a new capability means implementing one trait. Removing one means not registering it.

## The Tool Trait

Every tool implements this trait from [[peko-core]]:

```rust
pub trait Tool: Send + Sync + 'static {
    /// Unique name used by the LLM to invoke this tool
    fn name(&self) -> &str;

    /// Human-readable description (included in system prompt)
    fn description(&self) -> &str;

    /// JSON Schema for the tool's parameters
    fn parameters_schema(&self) -> serde_json::Value;

    /// Whether this tool is currently usable (hardware present, etc.)
    fn is_available(&self) -> bool { true }

    /// Whether execution requires external confirmation
    fn is_dangerous(&self) -> bool { false }

    /// Execute the tool with the given arguments
    fn execute(&self, args: serde_json::Value)
        -> Pin<Box<dyn Future<Output = Result<ToolResult>> + Send + '_>>;
}
```

### Why these methods?

| Method | Purpose |
|---|---|
| `name()` | LLM uses this string to call the tool: `{"name": "screenshot"}` |
| `description()` | Injected into system prompt so the LLM knows what the tool does |
| `parameters_schema()` | JSON Schema — LLM uses this to generate valid arguments |
| `is_available()` | Runtime check — e.g., modem not found → SmsTool unavailable |
| `is_dangerous()` | Gates execution behind confirmation. See [[ReAct-Loop]] |
| `execute()` | Async execution returning a result string (or error) |

### Example Implementation

```rust
pub struct ScreenshotTool {
    display: Arc<Framebuffer>,
}

impl Tool for ScreenshotTool {
    fn name(&self) -> &str { "screenshot" }

    fn description(&self) -> &str {
        "Capture the current screen as a PNG image"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    fn execute(&self, _args: serde_json::Value)
        -> Pin<Box<dyn Future<Output = Result<ToolResult>> + Send + '_>>
    {
        Box::pin(async move {
            let buffer = self.display.capture()?;
            let png = encode_png(&buffer)?;
            let b64 = base64::encode(&png);
            Ok(ToolResult::image(b64, "image/png"))
        })
    }
}
```

## ToolRegistry

Manages registration, schema generation, and dispatch:

```rust
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self;

    /// Register a tool (panics on duplicate names)
    pub fn register(&mut self, tool: impl Tool);

    /// Generate JSON schemas for all available tools (for LLM prompt)
    pub fn schemas(&self) -> Vec<serde_json::Value>;

    /// Execute a tool by name with given arguments
    pub async fn execute(&self, name: &str, args: serde_json::Value)
        -> Result<ToolResult>;

    /// List names of all available tools
    pub fn available_tools(&self) -> Vec<&str>;
}
```

### Schema Generation

`schemas()` produces an array of JSON objects in the format expected by LLM APIs:

```json
[
  {
    "name": "screenshot",
    "description": "Capture the current screen as a PNG image",
    "input_schema": {
      "type": "object",
      "properties": {},
      "required": []
    }
  },
  {
    "name": "touch",
    "description": "Inject a touch event on the screen",
    "input_schema": {
      "type": "object",
      "properties": {
        "action": {"type": "string", "enum": ["tap", "long_press", "swipe"]},
        "x": {"type": "integer"},
        "y": {"type": "integer"}
      },
      "required": ["action", "x", "y"]
    }
  }
]
```

This array is passed to [[LLM-Providers|the provider]] as the `tools` parameter and also injected into the system prompt for local models that use XML-based tool definitions.

### Dispatch Flow

```
LLM returns: tool_call { name: "touch", args: {"action":"tap","x":540,"y":1200} }
                │
                ▼
ToolRegistry::execute("touch", args)
                │
                ▼
HashMap lookup: tools["touch"] → Arc<TouchTool>
                │
                ▼
TouchTool::execute(args).await
                │
                ▼
ToolResult { content: "Tapped at (540, 1200)", is_error: false }
```

## ToolResult

```rust
pub struct ToolResult {
    pub content: String,       // Text result for the LLM
    pub is_error: bool,        // Whether this is an error
    pub image: Option<ImageData>, // Optional image for multimodal
}

pub struct ImageData {
    pub base64: String,
    pub media_type: String,  // "image/png"
}
```

Results are appended to the conversation as `Message::ToolResult` and sent back to the LLM on the next iteration.

## Available vs Registered

A tool can be registered but **unavailable** at runtime:

```rust
impl Tool for SmsTool {
    fn is_available(&self) -> bool {
        self.modem.is_connected()  // false if no modem found
    }
}
```

`schemas()` only includes tools where `is_available()` returns `true`. This means the LLM never sees tools it can't use — no wasted context tokens.

## Adding a New Tool

To add a capability to the agent:

1. Create a struct implementing `Tool` in [[peko-tools-android]]
2. Register it in [[peko-agent-binary|main.rs]]:
   ```rust
   registry.register(MyNewTool::new());
   ```
3. Done. The LLM automatically sees the new tool's schema.

No changes needed to [[peko-core]], [[peko-transport]], or any other crate.

## Related

- [[ReAct-Loop]] — Where tools get called
- [[peko-tools-android]] — Concrete tool implementations
- [[peko-core]] — Where the trait lives
- [[LLM-Providers]] — How schemas are sent to providers

---

#implementation #tools #trait #architecture
