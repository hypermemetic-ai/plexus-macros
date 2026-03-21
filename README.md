# plexus-macros

Procedural macros for Plexus RPC activations - **the source of truth** for the entire client generation pipeline.

---

## Overview: Macros → Client Libraries

These macros are the **entry point** for the complete Plexus code generation pipeline:

```
#[hub_methods] Rust Macros (this crate)
  ↓ schemars::schema_for!()
JSON Schema (PluginSchema + MethodSchema)
  ↓ Synapse (Haskell) fetches via WebSocket
Synapse IR (deduplicated, compiler-ready)
  ↓ hub-codegen (Rust) generates code
TypeScript/Rust Client Libraries
  ↓ synapse-cc (Haskell) integrates
Production-Ready Clients (type-safe, documented)
```

**Critical:** Everything downstream (TypeScript clients, Python clients, OpenAPI docs) depends on what these macros expose in the PluginSchema. The macro is the **source of truth** for:
- Type structure (enums, structs, fields)
- Method signatures (params, returns, streaming)
- Documentation (descriptions, examples)
- Validation rules (constraints, formats)
- API metadata (HTTP methods, tags, deprecation)

See [PIPELINE_OVERVIEW.md](./PIPELINE_OVERVIEW.md) for the complete flow.

---

## What is Plexus RPC?

Plexus RPC is a protocol for building services with runtime schema introspection. Unlike traditional RPC systems that require separate schema definitions, Plexus RPC extracts schemas directly from your code. This ensures zero drift between your implementation and schema.

**Key principle:** Your Rust function signature **is** the schema - no separate IDL files needed.

---

## Current State (hub_methods v1)

### Quick Start

```rust
use plexus::prelude::*;

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum BashEvent {
    Stdout { data: String },
    Stderr { data: String },
    Exit { code: i32 },
}

pub struct BashActivation {
    // your state
}

#[hub_methods(namespace = "bash", version = "1.0.0")]
impl BashActivation {
    /// Execute a bash command and stream output
    #[hub_method]
    async fn execute(&self, command: String, timeout: u64)
        -> impl Stream<Item = BashEvent> + Send + 'static
    {
        // implementation
    }
}
```

**This generates:**
- `BashMethod` enum for schema extraction
- `Activation` trait implementation
- RPC dispatch logic
- Schema introspection endpoint
- Constants: `NAMESPACE`, `VERSION`, `PLUGIN_ID`

---

## Current Macro Attributes

### `#[hub_methods]` - Impl Block Level

**Required:**
- `namespace = "..."` - The activation namespace

**Optional:**
- `version = "..."` - Semantic version (default: CARGO_PKG_VERSION)
- `description = "..."` - Short description (max 15 words)
- `long_description = "..."` - Detailed documentation
- `crate_path = "..."` - Import path (default: "crate")
- `hub` - This activation has child activations
- `resolve_handle` - Generate handle resolution method
- `plugin_id = "..."` - Explicit UUID (otherwise auto-generated)

### `#[hub_method]` - Method Level

**Optional:**
- `name = "..."` - Override method name
- `streaming` - Method yields multiple events over time
- `http_method = "GET|POST|PUT|DELETE|PATCH"` - HTTP verb for REST API
- `params(name = "description", ...)` - Parameter descriptions (awkward!)
- `returns(Variant1, Variant2, ...)` - Filter enum variants
- `bidirectional` - Method uses bidirectional channel

---

## Problems with Current Macros

See [MACRO_ANALYSIS.md](./MACRO_ANALYSIS.md) for detailed analysis. Key issues:

1. **Too much implicit behavior** - Magic parameter skipping (ctx), auto-injected schema method
2. **Poor naming** - "hub" is overloaded, generated names are hidden
3. **Complex implementation** - 1,825 lines with deeply nested parsing
4. **Weak metadata** - Missing doc comments, examples, validation constraints
5. **Awkward syntax** - `params(name = "desc")` is verbose and error-prone

**Result:** Clients get minimal documentation and no validation!

---

## Proposed Macro v2 Syntax

See [TARGET_DX.md](./TARGET_DX.md) for complete specification. Key improvements:

### Explicit, Clear, Type-Safe

```rust
#[activation(
    namespace = "bash",
    version = "1.0.0",
    description = "Execute shell commands",
    methods_enum = "BashMethods",   // Explicit generated name
    schema_method = true,           // Opt-in to auto-generated schema method
)]
impl BashActivation {
    /// Execute a bash command and stream output
    ///
    /// This runs commands in a bash shell and streams stdout/stderr
    /// as it becomes available.
    ///
    /// # Examples
    ///
    /// ```json
    /// {
    ///   "command": "ls -la",
    ///   "timeout": 30
    /// }
    /// ```
    #[method(
        stream = Multi,            // Explicit: yields multiple events
        http = POST,               // Type-safe enum, not string
        responses(
            ok(BashEvent, "Command output"),
            error(BashError, "Execution failed"),
        ),
        tags = ["shell", "execution"],
    )]
    async fn execute(
        &self,

        /// Shell command to execute
        #[example("ls -la")]
        #[validate(length(min = 1, max = 1000))]
        command: String,

        /// Maximum execution time in seconds
        #[default(30)]
        #[validate(range(min = 1, max = 300))]
        timeout: u64,

        #[skip]  // Explicit: not in schema
        ctx: &ExecutionContext,
    ) -> impl Stream<Item = BashEvent> { }
}
```

**Benefits:**
- Doc comments → descriptions (like clap)
- Inline parameter docs (no duplication)
- Validation rules declared (like validator)
- Examples for documentation and tests
- Everything explicit (no magic)
- Strongly-typed enums everywhere

**Generated TypeScript:**
```typescript
/**
 * Execute a bash command and stream output
 *
 * This runs commands in a bash shell and streams stdout/stderr
 * as it becomes available.
 *
 * @example
 * ```typescript
 * for await (const event of client.bash.execute("ls -la", 30)) {
 *   if (isBashEventStdout(event)) {
 *     console.log(event.data);
 *   }
 * }
 * ```
 *
 * @param command - Shell command to execute (example: "ls -la")
 * @param timeout - Maximum execution time in seconds (default: 30)
 * @returns Stream of bash events
 * @tags shell, execution
 */
execute(command: string, timeout?: number): AsyncGenerator<BashEvent>;
```

See [DX_INSPIRATION.md](./DX_INSPIRATION.md) for patterns borrowed from clap, serde, axum, utoipa, and validator.

---

## What Must Flow Through the Pipeline

For client libraries to be fully type-safe and well-documented, the macro must extract:

### Type Structure (✅ Currently Working)

- ✅ Enum discriminators (which field is the tag)
- ✅ Variant names and fields
- ✅ Format hints (uuid, int32, uint64, date-time)
- ✅ Required vs optional fields
- ✅ Type references (cross-namespace imports)
- ✅ Streaming semantics (AsyncGenerator vs Promise)
- ✅ HTTP method for REST endpoints

### Documentation (❌ Currently Missing)

- ❌ Method descriptions (from doc comments)
- ❌ Long descriptions (multiple paragraphs)
- ❌ Parameter descriptions (inline docs)
- ❌ Examples (JSON/code blocks)
- ❌ Deprecation warnings
- ❌ Tags for grouping

### Validation (❌ Currently Missing)

- ❌ Email/URL/UUID format validation
- ❌ String length constraints
- ❌ Numeric range constraints
- ❌ Regex pattern matching
- ❌ Enum value constraints
- ❌ Default values

### Metadata (❌ Currently Missing)

- ❌ Response documentation (success/error cases)
- ❌ Security requirements (auth level)
- ❌ Performance hints (caching, rate limits)
- ❌ Sensitive field markers (don't log)

**These must be added to PluginSchema/MethodSchema** for downstream tools to generate rich clients.

---

## Schema Introspection

Methods generated by `#[hub_methods]` are automatically discoverable via Plexus RPC's schema introspection:

```bash
# Using the synapse CLI
synapse bash {backend}.schema

# Or via raw JSON-RPC
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "bash.schema",
  "params": {}
}
```

This returns the complete `PluginSchema` with all methods, extracted from your Rust types.

---

## Integration with Plexus Ecosystem

This crate is part of the Plexus RPC ecosystem:

### Core Infrastructure
- **plexus-core** - Core `Activation` trait, streaming protocol
- **plexus-macros** - This crate - procedural macros
- **plexus-transport** - WebSocket, HTTP REST, MCP transports

### Code Generation Pipeline
- **synapse** (Haskell) - Schema fetcher, IR generator
- **hub-codegen** (Rust) - Stateless code generator (TypeScript, Rust)
- **synapse-cc** (Haskell) - Orchestrator (merge, deps, build)

### Clients & Tools
- **synapse CLI** - Interactive CLI for Plexus backends
- **Generated clients** - TypeScript, Python, Rust (from hub-codegen)

See [PIPELINE_OVERVIEW.md](./PIPELINE_OVERVIEW.md) for how these pieces fit together.

---

## Documentation Files

- **[README.md](./README.md)** - This file (overview + usage)
- **[MACRO_ANALYSIS.md](./MACRO_ANALYSIS.md)** - Detailed analysis of current macro problems
- **[TARGET_DX.md](./TARGET_DX.md)** - Complete specification for macro v2
- **[DX_INSPIRATION.md](./DX_INSPIRATION.md)** - Patterns from clap, serde, axum, etc.
- **[TYPE_EXTRACTION.md](./TYPE_EXTRACTION.md)** - How types flow through schemars
- **[PIPELINE_OVERVIEW.md](./PIPELINE_OVERVIEW.md)** - Complete Rust → TypeScript flow

---

## Examples

See the substrate reference server for complete examples:
- `bash/` - Shell command execution with streaming
- `arbor/` - Conversation tree storage
- `cone/` - LLM orchestration with bidirectional channels

---

## Current Status

**v1 Macros (stable):** Working but limited metadata extraction
**v2 Macros (planned):** Explicit syntax, rich metadata, full documentation

The v2 macros will be developed as **siblings** in the same crate, allowing gradual migration:

```rust
// Both work simultaneously
#[hub_methods(namespace = "old")]    // v1 (deprecated)
impl OldService { }

#[activation(namespace = "new")]     // v2 (recommended)
impl NewService { }
```

Migration tooling (codemod) will be provided for automated conversion.

---

## Contributing

When adding features to the macros, ensure they:
1. **Preserve type fidelity** through the entire pipeline
2. **Extract all relevant metadata** into PluginSchema
3. **Generate clear error messages** with examples
4. **Maintain backward compatibility** where possible
5. **Update downstream tools** (synapse IR, hub-codegen)

The macro is the **source of truth** - everything flows from here!

---

## License

MIT
