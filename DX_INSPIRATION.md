# DX Inspiration from the Rust Ecosystem

This document explores patterns from successful Rust libraries that we can adapt for Plexus macros v2.

---

## 1. Clap - Doc Comments as First-Class Citizens

**What they do well:**
```rust
#[derive(Parser)]
struct Args {
    /// The command to execute
    #[arg(short, long)]
    command: String,

    /// Timeout in seconds
    #[arg(short, long, default_value = "30")]
    timeout: u64,
}
```

**Key insights:**
- Doc comments become help text automatically
- Attributes augment but don't replace docs
- Short and long flags derived from field names
- Defaults are inline and type-checked

---

### Adaptation for Plexus:

```rust
#[activation(namespace = "bash")]
impl BashActivation {
    /// Execute a bash command and stream output
    ///
    /// This method runs the provided command in a bash shell and streams
    /// the stdout/stderr output as it becomes available.
    ///
    /// # Examples
    ///
    /// ```json
    /// {
    ///   "command": "ls -la",
    ///   "timeout": 30
    /// }
    /// ```
    #[method(http = POST, stream = Multi)]
    async fn execute(
        &self,
        /// The bash command to execute
        command: String,

        /// Maximum execution time in seconds
        #[default(30)]
        timeout: u64,

        /// Working directory for command execution
        #[default("/tmp")]
        workdir: PathBuf,
    ) -> impl Stream<Item = BashEvent> { }
}
```

**What gets extracted:**
- Method description: First paragraph of doc comment
- Method long description: Rest of doc comment before examples
- Examples: Code blocks tagged with `json` become schema examples
- Parameter descriptions: Doc comments on parameters
- Parameter defaults: From `#[default(...)]` attribute

**Benefits:**
- Natural Rust documentation style
- cargo doc shows the same info as API docs
- Examples are inline and visible
- IDE hover shows full context

---

## 2. Serde - Smart Defaults and Attribute Composition

**What they do well:**
```rust
#[derive(Serialize, Deserialize)]
struct User {
    #[serde(rename = "userId")]
    user_id: String,

    #[serde(default)]
    active: bool,

    #[serde(skip_serializing_if = "Option::is_none")]
    email: Option<String>,

    #[serde(flatten)]
    metadata: HashMap<String, Value>,
}
```

**Key insights:**
- Attributes are composable (multiple per field work together)
- Smart defaults (`default` uses Default trait)
- Skip patterns are flexible
- Flattening for composition

---

### Adaptation for Plexus:

```rust
#[method]
async fn update_user(
    &self,

    /// User ID to update
    #[validate(uuid)]
    user_id: String,

    /// User email address
    #[validate(email)]
    #[example("user@example.com")]
    email: Option<String>,

    /// User status
    #[default(UserStatus::Active)]
    #[validate(oneof = ["active", "inactive", "banned"])]
    status: UserStatus,

    /// Additional metadata
    #[flatten]
    metadata: HashMap<String, Value>,

    #[skip]
    ctx: &RequestContext,
) -> Result<User> { }
```

**New attributes:**
- `#[validate(...)]` - Parameter validation (email, uuid, range, etc.)
- `#[example("...")]` - Example value for docs/testing
- `#[flatten]` - Merge nested structure into params
- Multiple attributes compose naturally

---

## 3. Axum - Type-Driven Extractors

**What they do well:**
```rust
async fn handler(
    Path(user_id): Path<String>,
    Query(params): Query<SearchParams>,
    Json(body): Json<CreateUser>,
    State(db): State<Database>,
) -> Result<Json<User>> { }
```

**Key insight:** Type signature drives behavior (no annotations needed).

---

### Adaptation for Plexus:

```rust
#[method]
async fn create_user(
    &self,

    // Normal parameters (from request body)
    name: String,
    email: String,

    // Extractors (special parameter types)
    Auth(token): Auth<BearerToken>,      // Extract from auth header
    RateLimit(limiter): RateLimit,       // Rate limiting state
    Trace(span): Trace,                  // Tracing context

    #[skip]
    db: &Database,                       // Injected dependency
) -> Result<User> { }
```

**Benefits:**
- Type-based behavior is clear
- No magic parameter name detection
- Easy to add new extractor types
- Composable with other attributes

**Implementation:** Could use newtype wrappers that get special treatment:
```rust
pub struct Auth<T>(pub T);
pub struct RateLimit(pub RateLimiter);
pub struct Trace(pub Span);
```

---

## 4. Utoipa - Rich OpenAPI Annotations

**What they do well:**
```rust
#[utoipa::path(
    get,
    path = "/users/{id}",
    responses(
        (status = 200, description = "User found", body = User),
        (status = 404, description = "User not found"),
    ),
    params(
        ("id" = String, Path, description = "User ID")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
async fn get_user(path: Path<String>) -> Result<Json<User>> { }
```

**Key insights:**
- Rich response documentation
- Multiple response types
- Security requirements
- Path parameter documentation

---

### Adaptation for Plexus:

```rust
#[method(
    http = GET,
    responses(
        ok(User, "User found successfully"),
        not_found("User not found"),
        unauthorized("Invalid authentication token"),
    ),
    security = bearer,
    tags = ["users", "public"],
)]
async fn get_user(
    &self,

    /// User ID to retrieve
    #[validate(uuid)]
    user_id: String,
) -> Result<User> { }
```

**New features:**
- `responses(...)` - Document possible response types
- `security = ...` - Authentication requirements
- `tags = [...]` - OpenAPI tags for grouping
- Better error documentation

---

## 5. Validator - Declarative Validation

**What they do well:**
```rust
#[derive(Validate)]
struct User {
    #[validate(email)]
    email: String,

    #[validate(length(min = 8, max = 100))]
    password: String,

    #[validate(range(min = 18, max = 120))]
    age: u8,

    #[validate(url)]
    website: Option<String>,

    #[validate(custom = "validate_username")]
    username: String,
}
```

**Key insights:**
- Validation is declarative, not imperative
- Common validators built-in
- Custom validators supported
- Validation happens automatically

---

### Adaptation for Plexus:

```rust
#[method]
async fn register_user(
    &self,

    /// User's email address
    #[validate(email)]
    email: String,

    /// Password (8-100 characters)
    #[validate(length(min = 8, max = 100))]
    password: String,

    /// User's age
    #[validate(range(min = 18, max = 120))]
    age: u8,

    /// Username (alphanumeric only)
    #[validate(regex = r"^[a-zA-Z0-9_]+$")]
    username: String,

    /// Profile URL
    #[validate(url)]
    website: Option<String>,
) -> Result<User> { }
```

**Built-in validators:**
- `email` - Valid email format
- `url` - Valid URL format
- `uuid` - Valid UUID format
- `length(min, max)` - String length
- `range(min, max)` - Numeric range
- `regex = "..."` - Pattern matching
- `oneof = [...]` - Enum validation
- `custom = "fn_name"` - Custom validator function

**Benefits:**
- Validation happens before method is called
- Clear error messages with field-level details
- Schema includes validation constraints
- Self-documenting (validation rules visible in code)

---

## 6. Rocket - Route Attributes

**What they do well:**
```rust
#[get("/users/<id>?<limit>&<offset>")]
fn get_user(id: String, limit: Option<usize>, offset: Option<usize>) -> Json<User> { }

#[post("/users", data = "<user>")]
fn create_user(user: Json<CreateUser>) -> Status { }
```

**Key insight:** Path and query params are extracted from URL pattern.

---

### Adaptation for Plexus:

While we don't need URL routing (REST bridge handles that), we could support:

```rust
#[method(
    http = GET,
    path = "/users/{user_id}/posts/{post_id}",  // Explicit path template
)]
async fn get_post(
    &self,

    /// User ID (from path)
    #[path]
    user_id: String,

    /// Post ID (from path)
    #[path]
    post_id: String,

    /// Maximum results (from query)
    #[query]
    limit: Option<usize>,
) -> Result<Post> { }
```

**Or keep it simpler:**
- All non-annotated params → JSON body
- `#[path]` params → URL path segments
- `#[query]` params → URL query string

---

## 7. Tokio - Convention Over Configuration

**What they do well:**
```rust
#[tokio::main]
async fn main() { }

#[tokio::test]
async fn test_something() { }
```

**Key insight:** Minimal syntax for common cases.

---

### Adaptation for Plexus:

```rust
// Minimal case: just use doc comments
#[activation(namespace = "simple")]
impl SimpleActivation {
    /// Get user by ID
    async fn get_user(&self, id: String) -> User { }

    // No attributes needed!
    // - Method name: get_user
    // - HTTP method: POST (default)
    // - Streaming: auto-detected from return type
    // - Description: from doc comment
}

// Complex case: explicit configuration
#[activation(namespace = "complex")]
impl ComplexActivation {
    /// Get user by ID
    #[method(
        http = GET,
        stream = Single,
        cache = 60,
        rate_limit = (100, per_minute),
    )]
    async fn get_user(
        &self,
        #[validate(uuid)] id: String,
    ) -> User { }
}
```

**Progressive disclosure:** Start simple, add attributes as needed.

---

## 8. SQLx - Compile-Time Verification

**What they do well:**
```rust
let user = sqlx::query_as!(
    User,
    "SELECT id, name, email FROM users WHERE id = $1",
    user_id
)
.fetch_one(&pool)
.await?;
```

**Key insight:** Compile-time checking of SQL queries against real database.

---

### Adaptation for Plexus:

```rust
#[method]
async fn execute_query(
    &self,

    /// SQL query to execute
    #[validate(sql)]  // ← Compile-time validation against schema
    query: String,
) -> Result<QueryResult> { }
```

**Future possibility:**
- Validate parameter types against MethodSchema at compile time
- Validate return types match declared schema
- Catch type mismatches before runtime

---

## Complete Example: Combining All Patterns

Here's what a fully-featured Plexus activation could look like:

```rust
use plexus::prelude::*;

/// User management service
///
/// Provides CRUD operations for user accounts with authentication,
/// validation, and rate limiting.
#[activation(
    namespace = "users",
    version = "2.0.0",
    description = "User management service",
    methods_enum = "UserMethods",
    schema_method = true,
)]
impl UserService {
    /// Get a user by ID
    ///
    /// Retrieves detailed information about a specific user account.
    /// Requires authentication.
    ///
    /// # Examples
    ///
    /// ```json
    /// {
    ///   "user_id": "550e8400-e29b-41d4-a716-446655440000"
    /// }
    /// ```
    ///
    /// # Returns
    ///
    /// User object with all fields populated.
    #[method(
        http = GET,
        stream = Single,
        responses(
            ok(User, "User found"),
            not_found("User does not exist"),
            unauthorized("Invalid authentication"),
        ),
        security = bearer,
        tags = ["users", "read"],
        cache = 60,  // Cache for 60 seconds
    )]
    async fn get_user(
        &self,

        /// User ID to retrieve
        #[validate(uuid)]
        #[example("550e8400-e29b-41d4-a716-446655440000")]
        user_id: String,

        /// Include deleted users in search
        #[default(false)]
        include_deleted: bool,

        Auth(token): Auth<BearerToken>,
        RateLimit(limiter): RateLimit,

        #[skip]
        db: &Database,
    ) -> Result<User, UserError> {
        // Implementation
    }

    /// Create a new user
    ///
    /// Creates a new user account with validation.
    ///
    /// # Examples
    ///
    /// ```json
    /// {
    ///   "email": "user@example.com",
    ///   "name": "John Doe",
    ///   "age": 25
    /// }
    /// ```
    #[method(
        http = POST,
        stream = Single,
        responses(
            created(User, "User created successfully"),
            bad_request("Invalid user data"),
            conflict("User already exists"),
        ),
        tags = ["users", "write"],
        rate_limit = (10, per_minute),
    )]
    async fn create_user(
        &self,

        /// User's email address
        #[validate(email)]
        #[example("user@example.com")]
        email: String,

        /// Full name
        #[validate(length(min = 2, max = 100))]
        name: String,

        /// User's age (must be 18+)
        #[validate(range(min = 18, max = 120))]
        age: u8,

        /// Password (8-100 characters, alphanumeric + special chars)
        #[validate(length(min = 8, max = 100))]
        #[validate(regex = r"^[a-zA-Z0-9!@#$%^&*]+$")]
        #[sensitive]  // Don't log this parameter
        password: String,

        /// Additional user metadata
        #[flatten]
        metadata: HashMap<String, Value>,

        Auth(token): Auth<AdminToken>,

        #[skip]
        db: &Database,
    ) -> Result<User, UserError> {
        // Implementation
    }

    /// Watch user activity in real-time
    ///
    /// Streams user activity events as they occur. This is an infinite
    /// stream that continues until the client disconnects.
    #[method(
        http = POST,
        stream = Infinite,
        responses(
            ok(UserEvent, "Activity event stream"),
            unauthorized("Invalid authentication"),
        ),
        security = bearer,
        tags = ["users", "streaming"],
    )]
    async fn watch_activity(
        &self,

        /// User ID to watch
        #[validate(uuid)]
        user_id: String,

        /// Event types to include
        #[validate(oneof = ["login", "logout", "update", "delete"])]
        event_types: Vec<String>,

        Auth(token): Auth<BearerToken>,

        #[skip]
        event_bus: &EventBus,
    ) -> impl Stream<Item = UserEvent> {
        // Implementation
    }

    /// Interactive chat with AI assistant
    ///
    /// Bidirectional communication for AI-powered user assistance.
    #[method(
        stream = Multi,
        bidir = Custom {
            request: ChatMessage,
            response: ChatReply,
        },
        responses(
            ok(AssistantEvent, "Chat session established"),
        ),
        tags = ["ai", "interactive"],
    )]
    async fn ai_chat(
        &self,

        /// Initial user message
        initial_message: String,

        /// Conversation context
        #[flatten]
        context: ConversationContext,

        #[bidir]
        channel: &BidirChannel<ChatMessage, ChatReply>,

        Auth(token): Auth<BearerToken>,

        #[skip]
        ai_service: &AIService,
    ) -> impl Stream<Item = AssistantEvent> {
        // Implementation
    }

    /// Internal helper (not exposed via RPC)
    #[method(internal = true)]
    async fn validate_user_data(&self, data: &UserData) -> Result<()> {
        // Validation logic
    }

    /// Deprecated method (still works but warns)
    #[method(http = GET)]
    #[deprecated(since = "2.0.0", note = "Use `get_user` instead")]
    async fn fetch_user(&self, id: String) -> User {
        self.get_user(id, false).await
    }
}
```

---

## Summary: New Attributes and Features

### Activation-Level Attributes

```rust
#[activation(
    namespace = "...",          // Required: activation namespace
    version = "...",            // Optional: version string
    description = "...",        // Optional: short description

    // Optional features
    children = true,            // Has child activations
    methods_enum = "...",       // Custom enum name
    schema_method = true,       // Auto-generate schema method

    // Advanced
    crate_path = "...",         // Import path
    plugin_id = "...",          // Explicit UUID
)]
```

### Method-Level Attributes

```rust
#[method(
    // HTTP configuration
    http = GET,                 // HTTP method (enum)
    path = "/users/{id}",       // URL path template

    // Streaming configuration
    stream = Multi,             // Streaming mode (Single/Multi/Infinite)

    // Documentation
    responses(
        ok(Type, "desc"),       // Success response
        error(Code, "desc"),    // Error responses
    ),
    tags = ["tag1", "tag2"],    // OpenAPI tags

    // Security
    security = bearer,          // Auth requirement

    // Performance
    cache = 60,                 // Cache TTL in seconds
    rate_limit = (100, per_minute),  // Rate limiting

    // Advanced
    internal = true,            // Not exposed via RPC
    deprecated = "...",         // Deprecation message
    custom_dispatch = true,     // Custom dispatch logic
)]
```

### Parameter-Level Attributes

```rust
async fn method(
    &self,

    // Documentation
    /// Doc comment becomes description

    // Validation
    #[validate(email)]          // Email validation
    #[validate(uuid)]           // UUID validation
    #[validate(url)]            // URL validation
    #[validate(length(min, max))]  // Length validation
    #[validate(range(min, max))]   // Numeric range
    #[validate(regex = "...")]  // Pattern matching
    #[validate(oneof = [...])]  // Enum validation
    #[validate(custom = "fn")]  // Custom validator

    // Configuration
    #[default(value)]           // Default value
    #[example("...")]           // Example for docs
    #[flatten]                  // Merge nested structure
    #[skip]                     // Exclude from schema
    #[sensitive]                // Don't log this param

    // Special parameter types
    #[path]                     // From URL path
    #[query]                    // From query string
    #[bidir]                    // Bidirectional channel

    param: Type,
) { }
```

### Type-Based Extractors

```rust
Auth(token): Auth<BearerToken>      // Authentication
RateLimit(limiter): RateLimit       // Rate limiting
Trace(span): Trace                  // Tracing context
Cache(cache): Cache                 // Cache handle
```

---

## Implementation Priority

1. **Phase 1: Core DX** (Must-have)
   - Doc comments → descriptions
   - `#[desc("...")]` on parameters
   - `#[validate(...)]` basic validators
   - `#[default(...)]` with type checking
   - `#[skip]` explicit parameter exclusion

2. **Phase 2: Rich Metadata** (Should-have)
   - `#[example("...")]` for documentation
   - `responses(...)` documentation
   - `tags = [...]` for grouping
   - Better error messages
   - Deprecation warnings

3. **Phase 3: Advanced Features** (Nice-to-have)
   - Type-based extractors (Auth, RateLimit, etc.)
   - Cache TTL configuration
   - Rate limiting attributes
   - Custom validation functions
   - OpenAPI/JSON Schema enhancements

4. **Phase 4: Compile-Time Validation** (Future)
   - Validate schemas at compile time
   - Type checking for validation rules
   - Dead code detection (unused methods)
   - Breaking change detection

---

## Next Steps

1. Prototype Phase 1 features
2. Test with real-world activations
3. Gather feedback on ergonomics
4. Iterate on syntax
5. Implement Phase 2-3 features
6. Write comprehensive documentation
7. Build migration tooling

The goal: Make Plexus macros feel as polished and thoughtful as clap, serde, and axum.
