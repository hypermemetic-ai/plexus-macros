# Complete Pipeline: Rust → JSON Schema → Synapse IR → Client Libraries

Understanding the full type extraction and code generation pipeline.

---

## The Complete Flow

```
Rust Backend (#[hub_methods])
  ↓ [schemars::schema_for!]
JSON Schema (PluginSchema + MethodSchema)
  ↓ [WebSocket RPC fetch via synapse]
Synapse IR (deduplicated, compiler-ready)
  ↓ [hub-codegen Rust binary]
Generated Code (TypeScript/Rust)
  ↓ [synapse-cc orchestrator]
Client Libraries (type-safe, deployed)
```

---

## Stage 1: Rust Macros → JSON Schema

### Input: Rust Code with Macros

```rust
use plexus::prelude::*;

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(tag = "event", rename_all = "snake_case")]
enum BashEvent {
    Stdout { data: String },
    Stderr { data: String },
    ExitCode { code: i32 },
}

#[hub_methods(namespace = "bash", version = "1.0.0")]
impl BashActivation {
    /// Execute a bash command
    #[hub_method(streaming)]
    async fn execute(&self, command: String, timeout: u64)
        -> impl Stream<Item = BashEvent>
    {
        // implementation
    }
}
```

### Output: JSON Schema (in PluginSchema)

```json
{
  "namespace": "bash",
  "version": "1.0.0",
  "methods": [{
    "name": "execute",
    "description": "Execute a bash command",
    "params": {
      "type": "object",
      "required": ["command", "timeout"],
      "properties": {
        "command": { "type": "string" },
        "timeout": { "type": "integer", "format": "uint64" }
      },
      "$defs": {}
    },
    "returns": {
      "oneOf": [
        {
          "type": "object",
          "required": ["event", "data"],
          "properties": {
            "event": { "const": "stdout" },
            "data": { "type": "string" }
          }
        },
        {
          "type": "object",
          "required": ["event", "data"],
          "properties": {
            "event": { "const": "stderr" },
            "data": { "type": "string" }
          }
        },
        {
          "type": "object",
          "required": ["event", "code"],
          "properties": {
            "event": { "const": "exit_code" },
            "code": { "type": "integer", "format": "int32" }
          }
        }
      ],
      "$defs": {
        "BashEvent": { /* full enum schema */ }
      }
    },
    "streaming": true,
    "bidirectional": false,
    "http_method": "Post"
  }]
}
```

**What gets preserved:**
- ✅ Field names and types
- ✅ Required vs optional fields
- ✅ Type references via `$ref`
- ✅ Format hints (uint64, int32, uuid, date-time)
- ✅ Enum discriminators (tag field)
- ✅ Enum variants with fields
- ✅ Streaming flag
- ✅ Descriptions from doc comments

**What gets lost:**
- ❌ Rust-specific types (Cow, Arc, Box)
- ❌ Lifetime annotations
- ❌ Impl trait details
- ❌ Generic type parameters (resolved to concrete types)

---

## Stage 2: JSON Schema → Synapse IR

### Input: PluginSchema (from WebSocket RPC)

Synapse connects to the live backend and calls:
```
ws://localhost:4444/rpc
→ bash.schema → PluginSchema { ... }
→ (recursively walk children)
```

### Conversion Logic (Haskell)

```haskell
-- synapse/src/Synapse/IR/Builder.hs

schemaToTypeRef :: Text -> Value -> TypeRef
schemaToTypeRef namespace schema = case schema of
  Object o -> case (lookupType o, lookupRef o, lookupArray o, lookupNullable o) of
    -- Named type reference
    (_, Just ref, _, _) -> RefNamed (extractQualifiedName namespace ref)

    -- Array type
    (_, _, Just items, _) -> RefArray (schemaToTypeRef namespace items)

    -- Nullable type (anyOf with null)
    (_, _, _, Just innerSchema) -> RefOptional (schemaToTypeRef namespace innerSchema)

    -- Primitive type with format
    (Just primType, _, _, _) -> RefPrimitive primType (lookupFormat o)

  -- Intentionally dynamic (schema: true)
  Bool True -> RefAny

  -- Schema gap/error
  _ -> RefUnknown

-- Extract enum definition
extractEnumDef :: Value -> TypeDef
extractEnumDef schema = case findOneOf schema of
  Just variants -> TypeDef
    { tdKind = KindEnum
        { keDiscriminator = findDiscriminator schema  -- e.g., "event"
        , keVariants = map extractVariant variants
        }
    }
```

### Output: Synapse IR (JSON)

```json
{
  "irVersion": "2.0",
  "irBackend": "substrate",
  "irHash": "a1b2c3d4e5f6a7b8",
  "irTypes": {
    "bash.BashEvent": {
      "tdName": "BashEvent",
      "tdNamespace": "bash",
      "tdKind": {
        "tag": "KindEnum",
        "keDiscriminator": "event",
        "keVariants": [
          {
            "vdName": "stdout",
            "vdFields": [
              {
                "fdName": "data",
                "fdType": { "tag": "RefPrimitive", "contents": ["string", null] },
                "fdRequired": true
              }
            ]
          },
          {
            "vdName": "stderr",
            "vdFields": [
              {
                "fdName": "data",
                "fdType": { "tag": "RefPrimitive", "contents": ["string", null] },
                "fdRequired": true
              }
            ]
          },
          {
            "vdName": "exit_code",
            "vdFields": [
              {
                "fdName": "code",
                "fdType": { "tag": "RefPrimitive", "contents": ["integer", "int32"] },
                "fdRequired": true
              }
            ]
          }
        ]
      }
    }
  },
  "irMethods": {
    "bash.execute": {
      "mdName": "execute",
      "mdFullPath": "bash.execute",
      "mdNamespace": "bash",
      "mdStreaming": true,
      "mdParams": [
        {
          "pdName": "command",
          "pdType": { "tag": "RefPrimitive", "contents": ["string", null] },
          "pdRequired": true
        },
        {
          "pdName": "timeout",
          "pdType": { "tag": "RefPrimitive", "contents": ["integer", "uint64"] },
          "pdRequired": true
        }
      ],
      "mdReturns": { "tag": "RefNamed", "contents": ["bash", "BashEvent"] },
      "mdBidirType": null
    }
  },
  "irPlugins": {
    "bash": ["execute"]
  }
}
```

**What gets preserved:**
- ✅ Enum discriminators explicit ("event" field)
- ✅ Variant names and fields
- ✅ Format hints (int32, uint64, uuid, etc.)
- ✅ Qualified type references (namespace.TypeName)
- ✅ Optional vs required fields
- ✅ Streaming flag
- ✅ Bidirectional channel types (if present)

**What gets lost:**
- ❌ JSON Schema `$defs` structure (flattened to global irTypes map)
- ❌ Original oneOf encoding (normalized to discriminated enum)

**Key improvement:** IR is **deduplicated** and **compiler-ready** (no $ref resolution needed).

---

## Stage 3: Synapse IR → Generated Code (hub-codegen)

### Input: IR JSON

hub-codegen reads the IR and generates language-specific code.

### TypeScript Generation

```rust
// hub-codegen/src/generator/typescript/types.rs

fn generate_enum(enum_def: &EnumDef) -> String {
    let discriminator = &enum_def.keDiscriminator;

    // Generate interface per variant
    let variant_interfaces = variants.iter().map(|variant| {
        let variant_name = to_pascal_case(&variant.vdName);
        let interface_name = format!("{}{}", type_name, variant_name);

        let fields = variant.vdFields.iter().map(|field| {
            format!("  {}: {};", field.fdName, type_ref_to_ts(&field.fdType))
        }).collect::<Vec<_>>().join("\n");

        format!(
            "export interface {} {{\n  {}: '{}';\n{}\n}}",
            interface_name, discriminator, variant.vdName, fields
        )
    }).collect::<Vec<_>>().join("\n\n");

    // Generate union type
    let union_type = format!(
        "export type {} = {};",
        type_name,
        variant_names.join(" | ")
    );

    // Generate type guards
    let type_guards = variants.iter().map(|variant| {
        format!(
            "export function is{}(e: {}): e is {} {{\n  return e.{} === '{}';\n}}",
            variant_name, type_name, interface_name, discriminator, variant.vdName
        )
    }).collect::<Vec<_>>().join("\n\n");

    format!("{}\n\n{}\n\n{}", variant_interfaces, union_type, type_guards)
}
```

### Output: TypeScript Client

```typescript
// bash/types.ts
export interface BashEventStdout {
  event: 'stdout';
  data: string;
}

export interface BashEventStderr {
  event: 'stderr';
  data: string;
}

export interface BashEventExitCode {
  event: 'exit_code';
  code: number;
}

export type BashEvent =
  | BashEventStdout
  | BashEventStderr
  | BashEventExitCode;

export function isBashEventStdout(e: BashEvent): e is BashEventStdout {
  return e.event === 'stdout';
}

export function isBashEventStderr(e: BashEvent): e is BashEventStderr {
  return e.event === 'stderr';
}

export function isBashEventExitCode(e: BashEvent): e is BashEventExitCode {
  return e.event === 'exit_code';
}

// bash/client.ts
export interface BashClient {
  execute(command: string, timeout: number): AsyncGenerator<BashEvent>;
}

export class BashClientImpl implements BashClient {
  constructor(private rpc: RpcClient) {}

  async *execute(command: string, timeout: number): AsyncGenerator<BashEvent> {
    const stream = await this.rpc.call<BashEvent>(
      'bash.execute',
      { command, timeout }
    );

    for await (const item of stream) {
      if (item.type === 'data') {
        yield item.content;
      } else if (item.type === 'error') {
        throw new Error(item.error);
      }
      // handle other PlexusStreamItem types
    }
  }
}
```

**What gets preserved:**
- ✅ Full discriminated union types
- ✅ Type guards for runtime discrimination
- ✅ Streaming → AsyncGenerator
- ✅ Non-streaming → Promise
- ✅ All field types and names
- ✅ Required vs optional (? suffix)

**What gets lost:**
- ❌ Format hints (uint64 → number, no runtime validation)
- ❌ Doc comments (not in IR yet)

---

## Stage 4: synapse-cc Integration

### What synapse-cc Adds

```
hub-codegen output (JSON mode)
  ↓
synapse-cc:
  ├─ Three-way merge (baseline ← current, new)
  ├─ Write files to output directory
  ├─ Detect package manager (bun/npm/yarn/pnpm)
  ├─ Install dependencies (npm add @types/node ws)
  ├─ Write tsconfig.json (if standalone)
  ├─ Run tsc --noEmit (if standalone)
  ├─ Run tests (if standalone && not --no-tests)
  └─ Write cache manifests
```

**Result:** Fully integrated, type-safe client library ready to use.

---

## What Type Information is Critical?

### Must Preserve (Currently Working)

1. **Enum discriminators** - Which field distinguishes variants
2. **Variant names and fields** - Full structure of each enum case
3. **Format hints** - uuid, int32, uint64, date-time
4. **Required vs optional** - Which fields can be missing
5. **Type references** - Cross-namespace imports
6. **Streaming flag** - AsyncGenerator vs Promise
7. **Bidirectional types** - Request/response channel types

### Should Preserve (Partially Working)

1. **Doc comments** - Currently not in IR (❌)
2. **Validation constraints** - min/max/pattern (❌)
3. **Example values** - For docs/tests (❌)
4. **Deprecation** - Warn about old methods (❌)
5. **Default values** - For optional params (❌)

### Could Preserve (Future)

1. **Newtype semantics** - UserId vs PostId distinction
2. **Branded types** - Runtime type tagging
3. **Custom validators** - Client-side validation
4. **Security hints** - Mark sensitive fields (#[sensitive])

---

## Current Pain Points

### 1. **Doc Comments Don't Flow Through**

```rust
/// Execute a bash command  // ← Lost!
#[hub_method]
async fn execute(&self, command: String) -> BashEvent { }
```

**Solution:** Add `description` field to IR MethodDef and TypeDef.

### 2. **No Parameter Descriptions**

```rust
#[hub_method(params(command = "Shell command to execute"))]
```

Currently awkward. With new macro:
```rust
#[method]
async fn execute(
    &self,
    /// Shell command to execute  // ← Should be in IR!
    command: String,
) { }
```

**Solution:** Extract param descriptions into IR ParamDef.

### 3. **Validation Constraints Lost**

```rust
#[derive(Validate, JsonSchema)]
struct CreateUser {
    #[validate(email)]
    email: String,  // ← Validation not in schema/IR!
}
```

**Solution:**
- Add schemars validation attributes
- Synapse extracts to IR
- hub-codegen emits validation in generated code

### 4. **No Example Values**

```rust
#[method]
async fn execute(
    &self,
    #[example("ls -la")] command: String,  // ← Not in IR
) { }
```

**Solution:** Add `examples` field to ParamDef and FieldDef in IR.

---

## Recommendations for Macro v2

### Add to PluginSchema (plexus-core)

```rust
pub struct MethodSchema {
    pub name: String,
    pub description: String,           // Already have ✅
    pub long_description: Option<String>,  // Add ⭐
    pub params: Option<Schema>,
    pub param_descriptions: HashMap<String, String>,  // Add ⭐
    pub param_examples: HashMap<String, Value>,       // Add ⭐
    pub returns: Option<Schema>,
    pub examples: Vec<String>,         // Add ⭐ (JSON examples)
    pub streaming: bool,
    pub http_method: HttpMethod,
    pub deprecated: Option<String>,    // Add ⭐
    pub tags: Vec<String>,             // Add ⭐
}
```

### Add to Synapse IR

```haskell
data ParamDef = ParamDef
  { pdName        :: Text
  , pdType        :: TypeRef
  , pdRequired    :: Bool
  , pdDescription :: Maybe Text      -- Add ⭐
  , pdExample     :: Maybe Value     -- Add ⭐
  , pdDefault     :: Maybe Value     -- Add ⭐
  }

data MethodDef = MethodDef
  { mdName            :: Text
  , mdDescription     :: Maybe Text      -- Add ⭐
  , mdLongDescription :: Maybe Text      -- Add ⭐
  , mdExamples        :: [Text]          -- Add ⭐ (JSON strings)
  , mdDeprecated      :: Maybe Text      -- Add ⭐
  , mdTags            :: [Text]          -- Add ⭐
  , -- ... existing fields
  }

data TypeDef = TypeDef
  { tdDescription :: Maybe Text          -- Add ⭐
  , -- ... existing fields
  }

data FieldDef = FieldDef
  { fdDescription :: Maybe Text          -- Add ⭐
  , fdExample     :: Maybe Value         -- Add ⭐
  , -- ... existing fields
  }
```

### Update hub-codegen Output

```typescript
// With full docs:

/**
 * Execute a bash command and stream output.
 *
 * This runs the command in a bash shell and streams stdout/stderr
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
 * @param command - Shell command to execute
 * @param timeout - Maximum execution time in seconds
 * @returns Stream of bash events
 * @deprecated Use executeV2 instead
 */
execute(command: string, timeout: number): AsyncGenerator<BashEvent>;
```

---

## Summary: Complete Type Flow

```
Rust Source
  → [schemars] →
JSON Schema (with $defs, oneOf, format hints)
  → [synapse schemaToTypeRef] →
Synapse IR (TypeRef, MethodDef, deduplicated)
  → [hub-codegen] →
TypeScript (discriminated unions, type guards, AsyncGenerator)
  → [synapse-cc] →
Client Library (type-safe, documented, validated)
```

**Every stage must preserve:**
1. Type structure (enums, structs, primitives)
2. Discriminators (which field is the tag)
3. Format hints (uuid, int32, etc.)
4. Required vs optional
5. Documentation (descriptions, examples)
6. Validation rules (min, max, pattern)
7. Streaming semantics
8. Deprecation warnings

**The macros are the SOURCE OF TRUTH.** Everything downstream depends on what they expose in PluginSchema!
