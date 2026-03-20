# Plexus Macro System Analysis

This document analyzes the current macro system (`#[hub_methods]` and `#[hub_method]`) to identify pain points, confusing patterns, and areas for improvement before building a replacement.

---

## Executive Summary

The current macro system works but has accumulated technical debt through:
1. **Implicit behavior** - too much magic, hard to predict what code gets generated
2. **Overly complex parsing** - deeply nested pattern matching with unclear flow
3. **Tight coupling** - parse → codegen → runtime tied together in non-obvious ways
4. **Poor naming** - "hub" terminology is confusing, method enum generation is hidden
5. **Fragile schema extraction** - runtime JSON manipulation of schemars output
6. **Hidden control flow** - attributes affect behavior in non-local ways

---

## Pain Points by Category

### 1. CONFUSING NAMING

#### `hub_methods` vs `hub_method` (Singular/Plural Confusion)
- **Current**: `#[hub_methods]` on impl, `#[hub_method]` on methods
- **Problem**: Plural/singular distinction is subtle and easy to miss
- **Better names**:
  - `#[activation_methods]` for the impl
  - `#[method]` for individual methods
  - Or even clearer: `#[plexus_impl]` / `#[rpc_method]`

#### "Hub" Terminology is Overloaded
```rust
#[hub_methods(hub)]  // hub = true means "has children"
```
- **Problem**: "hub" appears in macro name AND as a boolean flag with different meanings
- **Confusion**: Is this a "hub method" or does it "have a hub"?
- **Better**: `#[hub_methods(has_children)]` or `hierarchical`

#### "Activation" is Generic and Opaque
- **Problem**: What is an "Activation"? Not self-explanatory
- **Better**: `Service`, `RpcService`, `Plugin`, or `Component`
- The term "activation" makes sense in Substrate context but not as a general concept

#### Method Enum Generation is Hidden
```rust
#[hub_methods(namespace = "bash")]
impl Bash { ... }

// SECRETLY GENERATES:
enum BashMethod { ... }
```
- **Problem**: You don't know `BashMethod` exists until you read the docs or expanded macro
- **No visibility**: Generated names aren't mentioned in the attribute
- **Better**: Make it explicit: `#[hub_methods(method_enum = "BashMethod")]` or show it in docs

---

### 2. OVERLY IMPLICIT BEHAVIOR

#### Automatic `schema` Method Injection
```rust
// File: method_enum.rs:301-307
// Add the auto-generated schema method
let schema_method = MethodSchema::new(
    "schema".to_string(),
    "Get plugin or method schema...",
    "auto_schema".to_string(),
);
methods.push(schema_method);
```
- **Problem**: Every activation automatically gets a `schema` method you didn't write
- **Hard to discover**: Not mentioned in the trait or impl block
- **Conflicts**: What if you want to define your own `schema` method?
- **Better**: Make it explicit via attribute: `#[hub_methods(generate_schema_method)]`

#### Parameter Skipping Logic is Magical
```rust
// File: parse.rs:347-354
if name_str == "ctx" || name_str == "context" {
    if let Some(bidir_types) = extract_bidir_channel_types(&pat_type.ty) {
        bidirectional = bidir_types;
        // Don't include ctx in params (it's provided by framework)
        continue;  // ← MAGIC: Parameter disappears!
    }
}
```
- **Problem**: Parameters named `ctx` or `context` are automatically removed from the schema
- **Action at a distance**: Naming affects code generation in non-obvious ways
- **Hard to debug**: Why isn't my parameter showing up in the schema?
- **Better**: Explicit attribute: `#[hub_method(skip_param = "ctx")]` or `#[framework_provided]`

#### Bidirectional Inference is Too Smart
```rust
// File: parse.rs:411-444
// Detect ctx: &BidirChannel<Req, Resp> or ctx: &StandardBidirChannel
fn extract_bidir_channel_types(ty: &Type) -> Option<BidirType>
```
- **Problem**: Type signature of `ctx` parameter determines bidirectional behavior
- **Too implicit**: You have to know that `ctx: &BidirChannel<A, B>` triggers special codegen
- **Fragile**: Renaming `BidirChannel` breaks detection
- **Better**: Explicit attribute with clear validation

#### Streaming Detection from Return Type
```rust
// File: parse.rs:502-522
// Extract Item type from `impl Stream<Item = T>`
fn extract_stream_item_type(ty: &Type) -> Option<Type>
```
- **Problem**: Macro infers streaming from `impl Stream<Item = X>` in return type
- **Implicit**: Return type affects generated RPC behavior in non-obvious ways
- **Confusion**: Why do I need `#[hub_method(streaming)]` if it detects streams already?
- **Answer**: The `streaming` flag means "multiple events over time" vs "single result stream"
- **Better**: Make the distinction clearer: `#[multi_event]` vs detecting stream wrapper

---

### 3. OVERLY COMPLEX CODE

#### Deeply Nested Pattern Matching in Parsing
```rust
// File: parse.rs:52-155 (100+ lines in a single match block)
for meta in metas {
    match meta {
        Meta::Path(path) => { ... }
        Meta::NameValue(MetaNameValue { path, value, .. }) => {
            if path.is_ident("name") { ... }
            else if path.is_ident("http_method") { ... }
        }
        Meta::List(MetaList { path, tokens, .. }) => {
            if path.is_ident("params") {
                let parser = Punctuated::<Meta, Token![,]>::parse_terminated;
                let nested = syn::parse::Parser::parse2(parser, tokens.clone())?;
                for meta in nested {
                    if let Meta::NameValue(...) { ... }
                }
            } else if path.is_ident("returns") { ... }
            else if path.is_ident("bidirectional") { ... }
        }
    }
}
```
- **Problem**: 4-5 levels of nesting, hard to follow control flow
- **Hard to maintain**: Adding new attributes requires navigating deeply nested matches
- **Better**: Extract into separate functions per attribute type

#### Schema Extraction via JSON Manipulation
```rust
// File: method_enum.rs:222-259
// Extract oneOf variants from the schema
let one_of = schema_value
    .get("oneOf")
    .and_then(|v| v.as_array())
    .cloned()
    .unwrap_or_default();

let params = one_of.get(i).and_then(|variant| {
    variant
        .get("properties")
        .and_then(|props| props.get("params"))
        .cloned()
        .and_then(|mut p| {
            if let (Some(params_obj), Some(defs)) = (p.as_object_mut(), &root_defs) {
                params_obj.insert("$defs".to_string(), defs.clone());
            }
            serde_json::from_value::<schemars::Schema>(p).ok()
        })
});
```
- **Problem**: Runtime JSON manipulation to extract schema from generated enum
- **Fragile**: Depends on exact schemars output format (could break with updates)
- **Performance**: Serialize → manipulate → deserialize on every method call
- **Hard to debug**: JSON path traversal errors are runtime, not compile-time
- **Better**: Direct schema construction instead of round-tripping through JSON

#### Variant Filtering is Convoluted
```rust
// File: method_enum.rs:312-367 (55 lines!)
fn filter_return_schema(schema: schemars::Schema, allowed_variants: &[&str]) -> schemars::Schema {
    let mut schema_value = serde_json::to_value(&schema).expect(...);

    if let Some(one_of) = schema_value.get_mut("oneOf").and_then(|v| v.as_array_mut()) {
        one_of.retain(|variant| {
            let variant_name = variant
                .get("properties")
                .and_then(|props| props.get("type"))
                .and_then(|type_prop| type_prop.get("const"))
                .and_then(|c| c.as_str())
                .or_else(|| { ... });  // Fallback logic

            // Convert snake_case to PascalCase...
            let pascal_name = name.split('_').map(...).collect();

            allowed_variants.contains(&pascal_name.as_str()) ||
            allowed_variants.contains(&name)
        });
    }

    serde_json::from_value(schema_value).expect(...)
}
```
- **Problem**: 55 lines to filter enum variants from a schema
- **Complexity**: Multiple fallback paths for finding variant names
- **Fragile**: Assumes specific JSON schema structure
- **Better**: Build filtered schema directly, don't manipulate JSON at runtime

---

### 4. HARD TO SEE / HIDDEN BEHAVIOR

#### Generated Code is Not Obvious
```rust
#[hub_methods(namespace = "bash")]
impl Bash {
    #[hub_method]
    async fn execute(&self, command: String) -> impl Stream<Item = BashEvent> { }
}
```

**What actually gets generated** (not visible without cargo-expand):
```rust
// 1. Method enum
enum BashMethod {
    #[serde(rename = "execute")]
    Execute { command: String }
}

// 2. Trait implementation
impl Activation for Bash { ... }

// 3. RPC trait
#[rpc(server, namespace = "bash")]
trait BashRpc { ... }

// 4. RPC server implementation
impl BashRpcServer for Bash { ... }

// 5. Constants
impl Bash {
    pub const NAMESPACE: &'static str = "bash";
    pub const PLUGIN_ID: Uuid = uuid!(...);
}
```

- **Problem**: You write 10 lines, get 300+ lines of generated code
- **Hard to predict**: What names will be generated?
- **Hard to reference**: Can you import `BashMethod` from another module?
- **Better**: Show generated names in attribute or provide configuration

#### Dispatch Logic is Black Box
```rust
// File: activation.rs:29-60
let dispatch_arms = generate_dispatch_arms(methods, namespace, crate_path);
```
- **Problem**: The `call()` method implementation is completely generated
- **No control**: Can't customize dispatch logic without forking the macro
- **Hard to debug**: Runtime errors in dispatch are hard to trace back to source
- **Better**: Generate trait with default impl that can be overridden

#### Hash Computation is Opaque
```rust
// File: method_enum.rs:384-421
fn compute_method_hash(method: &MethodInfo) -> String {
    let mut hasher = DefaultHasher::new();
    method.method_name.hash(&mut hasher);
    method.description.hash(&mut hasher);
    // ... hash params ...
    format!("{:016x}", hasher.finish())
}
```
- **Problem**: Method hashes are computed with no visibility
- **No control**: Can't customize or override hash strategy
- **Versioning**: Hash changes break cached schemas, but you can't see when/why
- **Better**: Make hash computation pluggable or document the algorithm clearly

---

### 5. POOR ERROR MESSAGES

#### Generic Parse Errors
```rust
// Current:
"hub_methods requires namespace = \"...\" attribute"
```
- **Problem**: Doesn't show what you actually wrote or suggest fixes
- **Better**: Show example of correct usage

#### Validation Happens Late
```rust
// File: parse.rs:380-393
// This error happens AFTER parsing succeeds!
if is_result_plexus_stream(&return_type) {
    return Err(syn::Error::new_spanned(
        &method.sig.output,
        format!("Method `{}` returns Result<PlexusStream, _> which bypasses...", fn_name)
    ));
}
```
- **Problem**: Parse succeeds, then validation fails during method info extraction
- **Confusing**: Why did it parse successfully if it's invalid?
- **Better**: Validate during parsing or make validation a separate explicit step

#### No Help for Common Mistakes
```rust
// Common mistake: Forgetting #[hub_method] on a method
impl Bash {
    async fn secret_method(&self) { }  // ← Not exposed, no warning!
}
```
- **Problem**: Silent skipping of methods without `#[hub_method]`
- **No warning**: Easy to forget the attribute and wonder why method isn't available
- **Better**: Warn about async methods without `#[hub_method]` or provide opt-out

---

### 6. TIGHT COUPLING

#### crate_path Configuration is Global
```rust
#[hub_methods(namespace = "foo", crate_path = "crate")]
```
- **Problem**: `crate_path` must be specified once and applies to ALL generated code
- **Inflexible**: Can't mix different crate paths for different methods
- **Better**: Default to `::plexus_core` and only override when needed

#### Method Enum Tightly Coupled to Schemars
```rust
// File: method_enum.rs:168-169
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum BashMethod { ... }
```
- **Problem**: Generated enum MUST use schemars for schema generation
- **Inflexible**: Can't use a different schema library or custom schema logic
- **Better**: Make schema generation pluggable or optional

#### Streaming Detection Coupled to Stream Trait
```rust
// File: parse.rs:502-522
fn extract_stream_item_type(ty: &Type) -> Option<Type> {
    if let Type::ImplTrait(impl_trait) = ty {
        for bound in &impl_trait.bounds {
            if let syn::TypeParamBound::Trait(trait_bound) = bound {
                let last_segment = trait_bound.path.segments.last()?;
                if last_segment.ident == "Stream" { ... }
```
- **Problem**: Only recognizes `Stream` trait by exact name
- **Fragile**: Renaming or re-exporting `Stream` breaks detection
- **Better**: Allow custom stream trait or make it configurable

---

### 7. VERSIONING AND COMPATIBILITY

#### Auto-Generated UUID is Not Stable
```rust
// File: activation.rs:161-170
let plugin_id_str = plugin_id.unwrap_or_else(|| {
    let major_version = version_for_uuid.split('.').next().unwrap_or("0");
    let name = format!("{}@{}", namespace, major_version);
    Uuid::new_v5(&Uuid::NAMESPACE_OID, name.as_bytes()).to_string()
});
```
- **Problem**: UUID generated from namespace + major version
- **Breaks**: Changing namespace OR major version changes the UUID
- **Handle routing**: Handles become invalid when plugin_id changes
- **Better**: Require explicit plugin_id or warn about stability

#### Method Hash Changes Break Caching
```rust
// Changing the description changes the hash!
method.description.hash(&mut hasher);
```
- **Problem**: Doc comment changes invalidate cached schemas
- **Too sensitive**: Non-semantic changes trigger cache misses
- **Better**: Hash only signature (name + params + return type), not docs

---

### 8. MISSING FEATURES / INFLEXIBILITY

#### No Way to Customize Method Enum Name
```rust
let enum_name = format_ident!("{}Method", struct_name);  // Always {Name}Method
```
- **Problem**: Enum name is always `{StructName}Method`
- **Conflicts**: What if that name is already taken?
- **Better**: Allow override: `#[hub_methods(method_enum = "CustomName")]`

#### No Way to Skip Schema Generation
```rust
// Always generates MethodSchema for every method
```
- **Problem**: Can't opt out of schema generation for internal methods
- **Workaround**: None - all methods get schemas
- **Better**: `#[hub_method(skip_schema)]` or `#[internal]`

#### No Async Trait Configuration
```rust
// File: activation.rs:199
#[async_trait::async_trait]
impl #impl_generics #rpc_server_name for #self_ty #where_clause {
```
- **Problem**: Always uses `async_trait`, even if you're using native async traits (Rust 1.75+)
- **Future incompatibility**: Will need to support both old and new async trait syntax
- **Better**: Detect Rust version or allow configuration

#### No Control Over Generated Documentation
```rust
/// Auto-generated method enum for schema extraction
```
- **Problem**: Generated code has minimal docs
- **Hard to use**: cargo doc doesn't explain what `BashMethod` is for
- **Better**: Generate rich documentation explaining the generated types

---

## Recommendations for New Macro System

### 1. Explicit Over Implicit
- **Required attributes**: Make behavior explicit, not inferred
- **Minimal magic**: Only generate what's absolutely necessary
- **Clear errors**: Explain what's wrong AND how to fix it

### 2. Flat Attribute Parsing
- **Separate parsers**: One function per attribute type
- **Early validation**: Validate during parsing, not later
- **Clear structure**: Dedicated structs for each attribute's data

### 3. Predictable Names
- **Show generated names**: Document or allow configuration
- **Avoid conflicts**: Namespace generated types clearly
- **Consistent naming**: `{Type}Methods` enum, `{Type}Rpc` trait, etc.

### 4. Direct Schema Construction
- **No JSON manipulation**: Build `MethodSchema` directly
- **Type-safe**: Use Rust types, not runtime JSON paths
- **Fast**: No serialize → modify → deserialize round trips

### 5. Configurable Defaults
- **Override everything**: Allow customization of all generated names
- **Sensible defaults**: Good default behavior that works 90% of the time
- **Progressive enhancement**: Simple case is simple, complex case is possible

### 6. Better Documentation
- **Explain generated code**: Document what the macro creates
- **Show examples**: Include before/after for common patterns
- **Error messages**: Link to docs from error messages

### 7. Modular Design
- **Separate concerns**: Parse, validate, codegen as distinct phases
- **Testable**: Each phase can be tested independently
- **Extensible**: Easy to add new attributes or features

---

## Example: What a Better Macro Might Look Like

```rust
// Current (implicit, magical):
#[hub_methods(namespace = "bash", hub)]
impl Bash {
    /// Execute a command
    #[hub_method(streaming)]
    async fn execute(&self, command: String) -> impl Stream<Item = BashEvent> {
        // ...
    }
}

// Better (explicit, clear):
#[plexus::rpc(
    namespace = "bash",
    has_children = true,  // Instead of mysterious "hub" flag
    method_enum = "BashMethods",  // Explicit name
    generate_schema_method = true,  // Opt-in to magic
)]
impl Bash {
    /// Execute a command
    #[rpc_method(
        streaming = true,  // Clear meaning
        http_method = HttpMethod::Post,  // Strongly typed
    )]
    async fn execute(
        &self,
        command: String,
        #[skip_schema] ctx: &Context,  // Explicit skip
    ) -> impl Stream<Item = BashEvent> {
        // ...
    }
}
```

**Advantages**:
- All generated names are explicit
- Flags have clear meanings
- Magic behavior is opt-in
- Parameters can be individually configured
- Strongly typed where possible

---

## Complexity Metrics

### Lines of Code
- `parse.rs`: 545 lines (attribute parsing + validation)
- `method_enum.rs`: 435 lines (enum generation + schema extraction)
- `activation.rs`: ~500 lines (trait impl + RPC generation)
- `lib.rs`: 345 lines (entry points + standalone helpers)

**Total: ~1,825 lines** of macro code for what the user sees as 2 simple attributes.

### Cyclomatic Complexity Hot Spots
1. `HubMethodAttrs::parse()` - 10+ branches in nested matches
2. `extract_bidir_channel_types()` - 6+ nested type checks
3. `filter_return_schema()` - 8+ JSON path variations
4. `compute_method_schemas()` - 15+ steps in schema extraction pipeline

### Hidden Dependencies
- Requires `schemars` for schema generation
- Requires `serde` + `serde_json` for JSON manipulation
- Requires `jsonrpsee` for RPC trait generation
- Requires `async_trait` for async trait impls
- Requires specific schemars output format (fragile)

---

## Conclusion

The current macro system **works** but is **hard to understand, modify, and debug**. The main issues are:

1. **Too much implicit behavior** - magic parameter skipping, automatic schema method injection
2. **Poor naming** - "hub" is overloaded, generated names are hidden
3. **Overly complex implementation** - deeply nested parsing, fragile JSON manipulation
4. **Tight coupling** - hardcoded dependencies on schemars, Stream trait, async_trait

A replacement macro system should prioritize:
- **Explicitness** - make all behavior visible and configurable
- **Simplicity** - flat attribute parsing, direct code generation
- **Flexibility** - allow customization of all generated names and behavior
- **Type safety** - use Rust types instead of JSON manipulation
- **Better errors** - clear messages with suggested fixes

The new system should be a **sibling** (not a replacement) initially, allowing gradual migration and A/B testing of the designs.
