# Target Developer Experience (DX) for Plexus Macros v2

This document defines the ideal syntax and behavior for the next-generation Plexus macro system, designed to address all pain points identified in MACRO_ANALYSIS.md.

---

## Design Principles

1. **Explicit over implicit** - All behavior must be visible and opt-in
2. **Predictable names** - All generated types must be configurable or obvious
3. **Progressive disclosure** - Simple cases are concise, complex cases are possible
4. **Strongly typed** - Use enums/types instead of strings wherever possible
5. **Self-documenting** - Attributes should clearly express intent
6. **Zero surprise** - No magic behavior or action-at-a-distance

---

## Core Syntax

### Simple Leaf Activation (No Children)

```rust
use plexus::prelude::*;

/// Execute bash commands
#[activation(
    namespace = "bash",
    version = "1.0.0",
)]
impl BashActivation {
    /// Execute a bash command and stream output
    #[method]
    async fn execute(&self, command: String) -> impl Stream<Item = BashEvent> {
        // implementation
    }

    /// List available shell builtins
    #[method(http = GET)]
    async fn list_builtins(&self) -> impl Stream<Item = Builtin> {
        // implementation
    }
}
```

**What gets generated:**
```rust
// 1. Method enum (name is predictable: {TypeName}Methods)
pub enum BashActivationMethods {
    Execute { command: String },
    ListBuiltins,
}

// 2. Activation trait impl
impl Activation for BashActivation {
    fn namespace(&self) -> &str { "bash" }
    fn version(&self) -> &str { "1.0.0" }
    fn schema(&self) -> PluginSchema { /* ... */ }
    async fn call(&self, method: &str, params: Value) -> Result<PlexusStream> {
        // dispatch to execute() or list_builtins()
    }
}

// 3. Constants
impl BashActivation {
    pub const NAMESPACE: &'static str = "bash";
    pub const VERSION: &'static str = "1.0.0";
    pub const PLUGIN_ID: Uuid = /* deterministic UUID */;
}
```

---

### Hub Activation (With Children)

```rust
#[activation(
    namespace = "tools",
    version = "1.0.0",
    children = true,  // Explicit: this activation has child activations
)]
impl ToolsHub {
    /// Route to child activation
    #[method]
    async fn call(&self, namespace: String, method: String, params: Value)
        -> impl Stream<Item = PlexusStreamItem>
    {
        self.route_to_child(&namespace, &method, params).await
    }
}
```

**Key difference:** `children = true` explicitly declares hub behavior (no mysterious "hub" flag).

---

## Attribute Reference

### `#[activation(...)]` - Top-Level Configuration

```rust
#[activation(
    namespace = "my_service",           // Required: activation namespace
    version = "1.0.0",                  // Optional: defaults to CARGO_PKG_VERSION
    description = "Short description",  // Optional: up to 15 words

    // Optional features:
    children = true,                    // This activation has child activations
    plugin_id = "550e8400-...",        // Explicit UUID (otherwise auto-generated)
    methods_enum = "CustomMethodsEnum", // Override generated enum name

    // Advanced:
    crate_path = "crate",              // Import path (default: "::plexus_core")
)]
impl MyService { ... }
```

**Generated types:**
- Enum: `{TypeName}Methods` (or custom via `methods_enum`)
- Trait: `Activation` (standard trait, always the same)
- Constants: `NAMESPACE`, `VERSION`, `PLUGIN_ID`

---

### `#[method(...)]` - Method Configuration

```rust
#[method(
    name = "custom_name",        // Optional: override method name
    http = GET,                  // Optional: HTTP method (enum, not string!)
    stream = Multi,              // Optional: streaming mode (see below)

    // Documentation:
    description = "Override doc comment",  // Optional: explicit description

    // Advanced:
    skip_schema = true,          // Don't generate schema for this method
    internal = true,             // Not exposed via RPC (internal helper)
)]
async fn my_method(&self, ...) { ... }
```

---

## Streaming Configuration

Instead of a boolean `streaming` flag, use an explicit enum:

```rust
use plexus::StreamMode;

#[method(stream = Single)]   // Yields exactly one event, then Done
async fn get_user(&self, id: String) -> impl Stream<Item = User> { ... }

#[method(stream = Multi)]    // Yields 0..N events over time
async fn watch_logs(&self) -> impl Stream<Item = LogEntry> { ... }

#[method(stream = Infinite)] // Never ends (e.g., event subscription)
async fn subscribe(&self) -> impl Stream<Item = Event> { ... }
```

**Benefits:**
- Clear intent (Single vs Multi vs Infinite)
- Better client codegen (Promise vs AsyncGenerator vs EventStream)
- Enables future optimizations (Single can use simpler transport)

---

## HTTP Method Configuration

Use strongly-typed enum instead of strings:

```rust
use plexus::HttpMethod;

#[method(http = GET)]          // Read operation
async fn get_user(&self, id: String) -> User { ... }

#[method(http = POST)]         // Create operation (default)
async fn create_user(&self, data: UserData) -> User { ... }

#[method(http = PUT)]          // Replace operation
async fn update_user(&self, id: String, data: UserData) -> User { ... }

#[method(http = DELETE)]       // Delete operation
async fn delete_user(&self, id: String) -> DeleteResult { ... }

#[method(http = PATCH)]        // Partial update
async fn patch_user(&self, id: String, patch: UserPatch) -> User { ... }
```

**Benefits:**
- Compile-time validation (can't misspell "GETT")
- IDE autocomplete works
- No uppercase/lowercase confusion
- Can't use invalid HTTP methods

---

## Parameter Configuration

Explicit control over parameter handling:

```rust
#[method]
async fn execute(
    &self,
    command: String,                        // Normal parameter (appears in schema)

    #[desc("Timeout in seconds")]
    timeout: Option<u64>,                   // Optional with custom description

    #[skip] ctx: &ExecutionContext,         // Explicitly skip from schema

    #[default(10)]
    max_retries: u32,                       // Parameter with default value
) -> Result<Output> { ... }
```

**Available parameter attributes:**
- `#[desc("...")]` - Add description to schema (appears in API docs)
- `#[skip]` - Exclude from schema (framework-provided params)
- `#[default(value)]` - Specify default value (makes param optional)
- `#[bidir]` - Mark as bidirectional channel parameter

**Examples of all parameter patterns:**

```rust
#[method]
async fn complex_method(
    &self,

    // Simple parameter (no annotation)
    user_id: String,

    // Parameter with description
    #[desc("Maximum results to return")]
    limit: usize,

    // Optional parameter with description
    #[desc("Filter by status")]
    status: Option<Status>,

    // Parameter with default value
    #[default(10)]
    page_size: usize,

    // Parameter with both description and default
    #[desc("Timeout in seconds")]
    #[default(30)]
    timeout: u64,

    // Framework-provided parameter (not in schema)
    #[skip]
    ctx: &RequestContext,

    // Bidirectional channel (marked explicitly)
    #[bidir]
    channel: &BidirChannel<Req, Resp>,
) -> Result<Output> { ... }
```

**Key features:**
- `#[skip]` makes parameter skipping explicit (no magic name-based detection)
- Descriptions are inline with parameters (no duplication, refactoring-friendly)
- Multiple attributes can be combined (e.g., `#[desc("...")] #[default(10)]`)

---

### Advanced: Doc Comments on Parameters (Future)

In the future, we could support extracting descriptions from doc comments:

```rust
#[method]
async fn execute(
    &self,
    /// The command to execute
    command: String,
    /// Timeout in seconds
    timeout: Option<u64>,
) -> Result<Output> { ... }
```

This would require rustc API support for parameter doc comments, which is currently unstable. Until then, `#[desc("...")]` is the recommended approach.

---

## Bidirectional Communication

Explicit configuration instead of type inference:

```rust
use plexus::BidirMode;

// Standard bidirectional (StandardRequest/StandardResponse)
#[method(bidir = Standard)]
async fn interactive(
    &self,
    initial: String,
    #[bidir] channel: &BidirChannel,  // Explicitly mark the channel param
) -> impl Stream<Item = Response> { ... }

// Custom bidirectional types
#[method(bidir = Custom {
    request: MyRequest,
    response: MyResponse,
})]
async fn custom_interactive(
    &self,
    #[bidir] channel: &BidirChannel<MyRequest, MyResponse>,
) -> impl Stream<Item = MyEvent> { ... }
```

**Benefits:**
- No type inference magic
- Clear which parameter is the channel
- Explicit request/response types

---

## Schema Introspection

Opt-in to auto-generated schema method:

```rust
#[activation(
    namespace = "bash",
    version = "1.0.0",
    schema_method = true,  // ← Explicit opt-in
)]
impl BashActivation { ... }
```

**Generated:**
```rust
impl BashActivation {
    /// Get plugin or method schema
    async fn schema(&self, request: SchemaRequest)
        -> impl Stream<Item = SchemaResponse>
    {
        // Auto-generated implementation
    }
}
```

**Benefits:**
- No surprise method injection
- Can opt-out if you want custom schema logic
- Clear in the code that schema method exists

---

## Error Handling

Explicit error type configuration:

```rust
#[activation(
    namespace = "bash",
    error = BashError,  // ← Explicit error type
)]
impl BashActivation {
    #[method]
    async fn execute(&self, cmd: String) -> Result<Output, BashError> {
        // BashError is converted to PlexusError automatically
    }
}
```

Without `error` attribute, methods must return `PlexusError` directly.

---

## Advanced: Custom Dispatch

Override the generated `call()` implementation:

```rust
#[activation(
    namespace = "custom",
    custom_dispatch = true,  // ← Opt-in to custom dispatch
)]
impl CustomActivation {
    #[method]
    async fn method_a(&self) -> Result<A> { ... }

    #[method]
    async fn method_b(&self) -> Result<B> { ... }

    // Custom dispatch logic (you implement this)
    async fn dispatch_call(
        &self,
        method: &str,
        params: Value,
    ) -> Result<PlexusStream, PlexusError> {
        // Your custom logic here
        // Can still call the generated dispatch as fallback:
        self.default_dispatch(method, params).await
    }
}
```

**Generated:**
```rust
impl Activation for CustomActivation {
    async fn call(&self, method: &str, params: Value)
        -> Result<PlexusStream, PlexusError>
    {
        self.dispatch_call(method, params).await  // Calls YOUR implementation
    }
}
```

---

## Examples: Common Use Cases

### 1. Simple CRUD Service

```rust
#[activation(namespace = "users", version = "1.0.0")]
impl UserService {
    #[method(http = GET)]
    async fn get(&self, id: String) -> User { ... }

    #[method(http = POST)]
    async fn create(&self, user: CreateUser) -> User { ... }

    #[method(http = PUT)]
    async fn update(&self, id: String, user: UpdateUser) -> User { ... }

    #[method(http = DELETE)]
    async fn delete(&self, id: String) -> DeleteResult { ... }
}
```

---

### 2. Streaming Service

```rust
#[activation(namespace = "logs", version = "1.0.0")]
impl LogService {
    /// Get historical logs (finite stream)
    #[method(stream = Multi)]
    async fn history(
        &self,
        #[desc("Start timestamp")] from: DateTime<Utc>,
        #[desc("End timestamp")] to: DateTime<Utc>,
    ) -> impl Stream<Item = LogEntry> { ... }

    /// Subscribe to live logs (infinite stream)
    #[method(stream = Infinite)]
    async fn subscribe(&self) -> impl Stream<Item = LogEntry> { ... }
}
```

---

### 3. Interactive Service (Bidirectional)

```rust
#[activation(namespace = "chat", version = "1.0.0")]
impl ChatService {
    #[method(
        bidir = Custom {
            request: ChatMessage,
            response: ChatReply,
        },
        stream = Multi,
    )]
    async fn conversation(
        &self,
        initial_prompt: String,
        #[bidir] channel: &BidirChannel<ChatMessage, ChatReply>,
    ) -> impl Stream<Item = ChatEvent> {
        // Send messages via channel.send(ChatReply { ... }).await
        // Receive messages via channel.recv() stream
    }
}
```

---

### 4. Hub with Children

```rust
#[activation(
    namespace = "tools",
    version = "1.0.0",
    children = true,
    schema_method = true,  // Enable discovery
)]
impl ToolsHub {
    /// List available child activations
    #[method(http = GET)]
    async fn list_children(&self) -> Vec<ChildInfo> {
        self.children()
            .map(|c| ChildInfo { namespace: c.namespace(), ... })
            .collect()
    }

    /// Get schema for a specific child
    #[method(http = GET)]
    async fn child_schema(&self, namespace: String) -> PluginSchema {
        self.child_by_namespace(&namespace)
            .map(|c| c.schema())
            .ok_or_else(|| PlexusError::NotFound)
    }
}
```

---

### 5. Internal Helper Methods

```rust
#[activation(namespace = "db", version = "1.0.0")]
impl DatabaseService {
    /// Public RPC method
    #[method]
    async fn query(&self, sql: String) -> QueryResult {
        self.validate_query(&sql)?;  // ← Internal helper
        self.execute_query(sql).await
    }

    /// Internal helper (not exposed via RPC)
    #[method(internal = true)]
    async fn validate_query(&self, sql: &str) -> Result<()> {
        // Validation logic
    }
}
```

---

## Parameter Documentation: Old vs New

### Current System (Awkward)

```rust
#[hub_method(params(
    command = "Command to execute",
    timeout = "Timeout in seconds",
    retries = "Number of retries"
))]
async fn execute(
    &self,
    command: String,
    timeout: Option<u64>,
    retries: usize,
) -> Result<Output> { ... }
```

**Problems:**
- Parameter names are repeated (once in params, once in signature)
- Easy to get out of sync (rename param → forget to update params)
- Descriptions are far from the parameters they document
- No IDE support (params is just a string map)
- Can't see param type when reading description

---

### New System (Inline & Clear)

```rust
#[method]
async fn execute(
    &self,
    #[desc("Command to execute")] command: String,
    #[desc("Timeout in seconds")] timeout: Option<u64>,
    #[desc("Number of retries")] retries: usize,
) -> Result<Output> { ... }
```

**Benefits:**
- Description right next to parameter (easy to read)
- No duplication (name appears once)
- IDE support (attributes on parameters)
- Refactoring-friendly (rename updates everything)
- Type visible next to description

---

## Comparison: Current vs Target DX

### Current (Implicit, Magical)

```rust
#[hub_methods(namespace = "bash", hub)]
impl Bash {
    #[hub_method(streaming, params(command = "Shell command to execute"))]
    async fn execute(
        &self,
        command: String,
        ctx: &ExecutionContext,  // ← Magically removed from schema
    ) -> impl Stream<Item = BashEvent> { ... }
}
```

**Problems:**
- `hub` flag is mysterious
- `streaming` is a boolean (what does true mean exactly?)
- `ctx` parameter disappears from schema with no indication
- Parameter docs are awkward: `params(command = "description")`
- Generated `BashMethod` enum is hidden
- Auto-injected `schema` method is surprising
- Can't override any names or behavior

---

### Target (Explicit, Clear)

```rust
#[activation(
    namespace = "bash",
    children = true,             // ← Clear: has child activations
    methods_enum = "BashMethods", // ← Explicit: generated enum name
    schema_method = true,        // ← Explicit: auto-generate schema method
)]
impl Bash {
    #[method(
        stream = Multi,          // ← Clear: yields multiple events
        http = POST,             // ← Clear: POST endpoint
    )]
    async fn execute(
        &self,
        #[desc("Shell command to execute")] command: String,  // ← Clean inline docs
        #[skip] ctx: &ExecutionContext,  // ← Explicit: skip from schema
    ) -> impl Stream<Item = BashEvent> { ... }
}
```

**Benefits:**
- All behavior is explicit and visible
- Enums instead of strings/booleans
- Generated names are predictable or configurable
- Magic features are opt-in
- Parameter handling is clear and inline
- Descriptions are next to the parameters they document

---

## Migration Path

The new macros will be **additive** (not breaking):

```rust
// Option 1: Use new macros alongside old ones
#[hub_methods(namespace = "old")]  // Still works
impl OldService { ... }

#[activation(namespace = "new")]   // New style
impl NewService { ... }

// Option 2: Feature flag for gradual migration
#[cfg_attr(feature = "legacy-macros", hub_methods(namespace = "service"))]
#[cfg_attr(not(feature = "legacy-macros"), activation(namespace = "service"))]
impl Service { ... }
```

**Deprecation timeline:**
1. Release v2 macros as `#[activation]` / `#[method]`
2. Mark `#[hub_methods]` / `#[hub_method]` as `#[deprecated]`
3. Provide migration guide and codemod tool
4. Remove old macros in next major version

---

## Code Generation Strategy

### Principle: Generate Less, Expose More

Instead of generating everything hidden:

```rust
// Current: Generates 300+ lines of hidden code
#[hub_methods(namespace = "bash")]
impl Bash { ... }

// Target: Generate minimal glue, use traits for behavior
#[activation(namespace = "bash")]
impl Bash { ... }
```

**What gets generated (target):**
1. **Method enum** - Only the enum itself, traits come from plexus_core
2. **Dispatch function** - Small match statement calling your methods
3. **Schema function** - Direct construction (no JSON manipulation)
4. **Constants** - NAMESPACE, VERSION, PLUGIN_ID

**What moves to plexus_core:**
- `MethodEnumSchema` trait with default impls
- Schema caching logic
- Common helpers (filtering, validation, etc.)

**Benefits:**
- Less generated code = easier to debug
- More reusable code = easier to maintain
- Clearer separation = easier to customize

---

## Type Safety Improvements

### 1. Strongly Typed Enums Everywhere

```rust
// Old: Stringly-typed
#[hub_method(http_method = "GET")]  // ← Can misspell

// New: Enum-typed
#[method(http = GET)]  // ← Compile-time checked
```

### 2. Explicit Modes Instead of Booleans

```rust
// Old: Boolean with unclear meaning
#[hub_method(streaming = true)]  // ← What does true mean?

// New: Explicit enum
#[method(stream = Multi)]  // ← Clear: multiple events over time
```

### 3. Compile-Time Validation

```rust
// Old: Runtime validation
#[hub_method(returns(Foo, Bar))]  // ← Typos only caught at runtime

// New: Type-checked
#[method(returns = [Foo, Bar])]   // ← Compiler checks Foo and Bar exist
```

---

## Documentation Generation

Auto-generate comprehensive docs for all generated code:

```rust
/// Execute bash commands
///
/// ## Generated Types
///
/// - **Methods Enum**: [`BashActivationMethods`]
/// - **Namespace**: `"bash"`
/// - **Version**: `"1.0.0"`
/// - **Plugin ID**: `550e8400-e29b-41d4-a716-446655440000`
///
/// ## Available Methods
///
/// - [`execute`](Self::execute) - Execute a bash command
/// - [`list_builtins`](Self::list_builtins) - List shell builtins
///
/// ## RPC Access
///
/// This activation implements the [`Activation`] trait and can be
/// accessed via any Plexus transport (WebSocket, HTTP, MCP).
#[activation(namespace = "bash", version = "1.0.0")]
impl BashActivation { ... }
```

**Benefits:**
- cargo doc shows all generated types
- Links to related items work
- Clear explanation of what the macro does

---

## Error Messages

### Before (Current)

```
error: hub_methods requires namespace = "..." attribute
  --> src/bash.rs:10:1
   |
10 | #[hub_methods]
   | ^^^^^^^^^^^^^^
```

### After (Target)

```
error: missing required attribute `namespace`
  --> src/bash.rs:10:1
   |
10 | #[activation]
   | ^^^^^^^^^^^^^ add namespace = "..." to configure this activation
   |
   = help: example: #[activation(namespace = "my_service", version = "1.0.0")]
   = note: the namespace identifies this activation in the Plexus system
```

**Improvements:**
- Shows what's missing
- Provides example fix
- Explains why the attribute is needed

---

## Summary: Key DX Improvements

| Aspect | Current | Target | Benefit |
|--------|---------|--------|---------|
| **Naming** | `#[hub_methods]` | `#[activation]` | Clear, not overloaded |
| **Streaming** | `streaming = true` | `stream = Multi` | Explicit intent |
| **HTTP Methods** | `http_method = "GET"` | `http = GET` | Type-safe |
| **Param Docs** | `params(x = "desc")` | `#[desc("desc")] x` | Inline, no duplication |
| **Param Skipping** | Magic name detection | `#[skip]` attribute | Explicit |
| **Param Defaults** | Not supported | `#[default(val)]` | Native support |
| **Children** | `hub` boolean | `children = true` | Self-documenting |
| **Schema Method** | Auto-injected | `schema_method = true` | Opt-in |
| **Generated Names** | Hidden | `methods_enum = "..."` | Configurable |
| **Bidirectional** | Type inference | `bidir = Standard` | Explicit |
| **Error Messages** | Generic | Contextual + examples | Helpful |
| **Documentation** | Minimal | Comprehensive | Discoverable |

---

## Next Steps

1. **Prototype implementation** - Build minimal working version
2. **Real-world testing** - Convert 2-3 existing activations to new syntax
3. **Gather feedback** - Iterate on ergonomics
4. **Migration tooling** - Build codemod to auto-convert old → new
5. **Documentation** - Write comprehensive guide + examples
6. **Stabilization** - Polish edge cases, improve error messages
7. **Deprecation** - Mark old macros deprecated after 1-2 releases

The goal is to have a system that is **obvious, flexible, and delightful** to use.
