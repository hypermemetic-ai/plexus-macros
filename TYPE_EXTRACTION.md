# Type Extraction for Strong Typing

How Plexus extracts and enforces types across the protocol boundary.

---

## 1. Basic Type Extraction

### Rust → JSON Schema → Client

```rust
#[method]
async fn create_user(
    &self,
    name: String,
    age: u32,
    active: bool,
    tags: Vec<String>,
    metadata: HashMap<String, Value>,
) -> User { }
```

**Extracted Schema:**
```json
{
  "params": {
    "type": "object",
    "required": ["name", "age", "active", "tags", "metadata"],
    "properties": {
      "name": { "type": "string" },
      "age": { "type": "integer", "format": "uint32" },
      "active": { "type": "boolean" },
      "tags": { "type": "array", "items": { "type": "string" } },
      "metadata": { "type": "object", "additionalProperties": true }
    }
  }
}
```

**TypeScript Client:**
```typescript
interface CreateUserParams {
  name: string;
  age: number;
  active: boolean;
  tags: string[];
  metadata: Record<string, any>;
}

await client.users.createUser({
  name: "Alice",
  age: 30,  // ✅ Type-checked!
  active: true,
  tags: ["admin"],
  metadata: { role: "superuser" }
});
```

---

## 2. Enum Extraction

### Rust Enum → Tagged Union

```rust
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
enum UserStatus {
    Active,
    Inactive { reason: String },
    Banned { until: DateTime<Utc>, reason: String },
}

#[method]
async fn update_status(&self, user_id: String, status: UserStatus) -> User { }
```

**Extracted Schema:**
```json
{
  "status": {
    "oneOf": [
      { "type": "object", "properties": { "type": { "const": "active" } } },
      { "type": "object", "properties": {
        "type": { "const": "inactive" },
        "reason": { "type": "string" }
      }},
      { "type": "object", "properties": {
        "type": { "const": "banned" },
        "until": { "type": "string", "format": "date-time" },
        "reason": { "type": "string" }
      }}
    ]
  }
}
```

**TypeScript Client:**
```typescript
type UserStatus =
  | { type: "active" }
  | { type: "inactive", reason: string }
  | { type: "banned", until: string, reason: string };

await client.users.updateStatus(userId, {
  type: "banned",
  until: "2024-12-31T00:00:00Z",
  reason: "Spam"
  // ✅ Discriminated union, fully type-checked!
});
```

---

## 3. Nested Struct Extraction

### Complex Types

```rust
#[derive(Serialize, Deserialize, JsonSchema)]
struct CreateUserRequest {
    email: String,
    profile: UserProfile,
    settings: UserSettings,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct UserProfile {
    name: String,
    bio: Option<String>,
    avatar_url: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct UserSettings {
    notifications: bool,
    theme: Theme,
}

#[derive(Serialize, Deserialize, JsonSchema)]
enum Theme { Light, Dark, Auto }

#[method]
async fn create_user(&self, req: CreateUserRequest) -> User { }
```

**Extracted Schema:**
```json
{
  "params": {
    "type": "object",
    "required": ["req"],
    "properties": {
      "req": {
        "type": "object",
        "required": ["email", "profile", "settings"],
        "properties": {
          "email": { "type": "string" },
          "profile": { "$ref": "#/$defs/UserProfile" },
          "settings": { "$ref": "#/$defs/UserSettings" }
        }
      }
    },
    "$defs": {
      "UserProfile": { /* ... */ },
      "UserSettings": { /* ... */ },
      "Theme": { "enum": ["Light", "Dark", "Auto"] }
    }
  }
}
```

**Benefits:**
- Full type tree extracted
- References resolved automatically
- Cyclic types supported

---

## 4. Return Type Extraction

### Stream Item Types

```rust
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(tag = "event", rename_all = "snake_case")]
enum BashEvent {
    Stdout { data: String },
    Stderr { data: String },
    ExitCode { code: i32 },
    Error { message: String },
}

#[method(stream = Multi)]
async fn execute(&self, command: String) -> impl Stream<Item = BashEvent> { }
```

**Extracted Schema:**
```json
{
  "returns": {
    "oneOf": [
      { "type": "object", "properties": {
        "event": { "const": "stdout" },
        "data": { "type": "string" }
      }},
      { "type": "object", "properties": {
        "event": { "const": "stderr" },
        "data": { "type": "string" }
      }},
      { "type": "object", "properties": {
        "event": { "const": "exit_code" },
        "code": { "type": "integer" }
      }},
      { "type": "object", "properties": {
        "event": { "const": "error" },
        "message": { "type": "string" }
      }}
    ]
  }
}
```

**TypeScript Client:**
```typescript
type BashEvent =
  | { event: "stdout", data: string }
  | { event: "stderr", data: string }
  | { event: "exit_code", code: number }
  | { event: "error", message: string };

for await (const event of client.bash.execute({ command: "ls" })) {
  switch (event.event) {
    case "stdout":
      console.log(event.data);  // ✅ data is string
      break;
    case "exit_code":
      console.log(event.code);  // ✅ code is number
      break;
  }
}
```

---

## 5. Validation from Types

### Using JsonSchema Validation Attributes

```rust
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use validator::Validate;

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
struct CreateUserRequest {
    #[validate(email)]
    #[schemars(regex(pattern = r"^[^@]+@[^@]+\.[^@]+$"))]
    email: String,

    #[validate(length(min = 2, max = 100))]
    #[schemars(length(min = 2, max = 100))]
    name: String,

    #[validate(range(min = 18, max = 120))]
    #[schemars(range(min = 18, max = 120))]
    age: u8,

    #[validate(url)]
    #[schemars(url)]
    website: Option<String>,
}

#[method]
async fn create_user(&self, req: CreateUserRequest) -> User {
    // Validation happens automatically before this runs!
}
```

**Extracted Schema with Validation:**
```json
{
  "email": {
    "type": "string",
    "format": "email",
    "pattern": "^[^@]+@[^@]+\\.[^@]+$"
  },
  "name": {
    "type": "string",
    "minLength": 2,
    "maxLength": 100
  },
  "age": {
    "type": "integer",
    "minimum": 18,
    "maximum": 120
  },
  "website": {
    "type": "string",
    "format": "uri"
  }
}
```

**Benefits:**
- Server validates before method runs
- Client validates before sending
- OpenAPI docs show constraints
- Both sides enforce same rules

---

## 6. Newtype Pattern for Domain Types

### Strong Typing with Zero Cost

```rust
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
struct UserId(String);

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
struct Email(String);

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
struct PostId(String);

#[method]
async fn get_user(&self, user_id: UserId) -> User { }

#[method]
async fn send_email(&self, to: Email, subject: String) -> EmailStatus { }

#[method]
async fn get_post(&self, post_id: PostId) -> Post { }
```

**Benefits:**
- Compile-time type safety (can't pass UserId where PostId expected)
- Runtime = plain string (zero cost)
- Schema = string (transparent to clients)
- Self-documenting (function signature shows intent)

**TypeScript Client:**
```typescript
type UserId = string & { readonly __brand: "UserId" };
type Email = string & { readonly __brand: "Email" };
type PostId = string & { readonly __brand: "PostId" };

// Can't mix them up:
const userId: UserId = "user-123" as UserId;
const postId: PostId = "post-456" as PostId;

await client.users.getUser(userId);     // ✅
await client.users.getUser(postId);     // ❌ Type error!
```

---

## 7. Generic Types

### Type Parameters Work

```rust
#[derive(Serialize, Deserialize, JsonSchema)]
struct PagedResponse<T> {
    data: Vec<T>,
    page: u32,
    total_pages: u32,
    total_items: u64,
}

#[method]
async fn list_users(&self, page: u32, limit: u32) -> PagedResponse<User> { }

#[method]
async fn list_posts(&self, page: u32, limit: u32) -> PagedResponse<Post> { }
```

**Extracted Schema:**
```json
{
  "returns": {
    "type": "object",
    "required": ["data", "page", "total_pages", "total_items"],
    "properties": {
      "data": { "type": "array", "items": { "$ref": "#/$defs/User" } },
      "page": { "type": "integer" },
      "total_pages": { "type": "integer" },
      "total_items": { "type": "integer" }
    }
  }
}
```

---

## 8. Result Types → Error Schemas

### Typed Errors

```rust
#[derive(Debug, Serialize, Deserialize, JsonSchema, thiserror::Error)]
#[serde(tag = "error", rename_all = "snake_case")]
enum UserError {
    #[error("User not found: {user_id}")]
    NotFound { user_id: String },

    #[error("Invalid email: {email}")]
    InvalidEmail { email: String },

    #[error("User already exists: {email}")]
    AlreadyExists { email: String },

    #[error("Database error: {message}")]
    DatabaseError { message: String },
}

#[method]
async fn create_user(&self, email: String) -> Result<User, UserError> {
    if !is_valid_email(&email) {
        return Err(UserError::InvalidEmail { email });
    }
    // ...
}
```

**Extracted Schema:**
```json
{
  "returns": {
    "type": "object",
    "oneOf": [
      { "properties": { "ok": { "$ref": "#/$defs/User" } } },
      { "properties": { "error": {
        "oneOf": [
          { "type": "object", "properties": {
            "error": { "const": "not_found" },
            "user_id": { "type": "string" }
          }},
          { "type": "object", "properties": {
            "error": { "const": "invalid_email" },
            "email": { "type": "string" }
          }}
        ]
      }}}
    ]
  }
}
```

**TypeScript Client:**
```typescript
type UserResult =
  | { ok: User }
  | { error: { error: "not_found", user_id: string } }
  | { error: { error: "invalid_email", email: string } }
  | { error: { error: "already_exists", email: string } }
  | { error: { error: "database_error", message: string } };

const result = await client.users.createUser({ email });

if ("ok" in result) {
  const user = result.ok;  // ✅ User type
} else {
  switch (result.error.error) {
    case "invalid_email":
      console.error(`Bad email: ${result.error.email}`);
      break;
    case "not_found":
      console.error(`Not found: ${result.error.user_id}`);
      break;
  }
}
```

---

## 9. Type Aliases for Reuse

### DRY Principles

```rust
pub type Timestamp = DateTime<Utc>;
pub type JsonValue = serde_json::Value;
pub type Metadata = HashMap<String, JsonValue>;

#[method]
async fn create_event(
    &self,
    name: String,
    timestamp: Timestamp,
    metadata: Metadata,
) -> Event { }
```

**Schema uses the resolved types:**
```json
{
  "timestamp": { "type": "string", "format": "date-time" },
  "metadata": { "type": "object", "additionalProperties": true }
}
```

---

## 10. Complete Example: E-Commerce API

```rust
#[derive(Serialize, Deserialize, JsonSchema)]
struct ProductId(String);

#[derive(Serialize, Deserialize, JsonSchema)]
struct Money {
    amount: f64,
    currency: Currency,
}

#[derive(Serialize, Deserialize, JsonSchema)]
enum Currency { USD, EUR, GBP }

#[derive(Serialize, Deserialize, JsonSchema)]
struct Product {
    id: ProductId,
    name: String,
    price: Money,
    in_stock: bool,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct CartItem {
    product_id: ProductId,
    quantity: u32,
}

#[derive(Serialize, Deserialize, JsonSchema, thiserror::Error)]
#[serde(tag = "error")]
enum CheckoutError {
    #[error("Product out of stock: {product_id}")]
    OutOfStock { product_id: ProductId },

    #[error("Insufficient funds")]
    InsufficientFunds { required: Money, available: Money },
}

#[activation(namespace = "shop")]
impl ShopActivation {
    #[method(http = GET)]
    async fn get_product(&self, id: ProductId) -> Option<Product> { }

    #[method(http = POST)]
    async fn checkout(
        &self,
        items: Vec<CartItem>,
    ) -> Result<OrderConfirmation, CheckoutError> { }
}
```

**Everything is type-safe:**
- ✅ Can't pass wrong ProductId type
- ✅ Money has currency attached
- ✅ Errors are discriminated unions
- ✅ Clients get full TypeScript types
- ✅ Validation happens automatically

---

## Summary: Type Safety Across Boundaries

| Rust Type | JSON Schema | TypeScript | Validation |
|-----------|-------------|------------|------------|
| `String` | `string` | `string` | ✅ |
| `u32`, `i64` | `integer` | `number` | ✅ |
| `f64` | `number` | `number` | ✅ |
| `bool` | `boolean` | `boolean` | ✅ |
| `Vec<T>` | `array` | `T[]` | ✅ |
| `Option<T>` | `T \| null` | `T \| undefined` | ✅ |
| `HashMap<K,V>` | `object` | `Record<K,V>` | ✅ |
| `enum` | `oneOf` | `union` | ✅ |
| `struct` | `object` | `interface` | ✅ |
| `Result<T,E>` | `oneOf` | `{ok:T}\|{error:E}` | ✅ |

**The protocol is strongly typed end-to-end!** 🎯
