//! The by-name params object a typed call sends.

use serde::Serialize;
use serde_json::{Map, Value};

use crate::error::ClientError;

/// Builds a call's params object from typed arguments — what a generated
/// client method does with its parameters before it calls the transport.
///
/// Keys are the method's parameter names, which are what its schema
/// declares. An `Option` parameter that is `None` is **omitted**, not sent
/// as `null` ([`optional`](Self::optional)): the shape a server with
/// `skip_serializing_if = "Option::is_none"` sends, and the one menger's
/// REST tests assert.
///
/// The first argument that does not serialize is kept as
/// [`ClientError::Encode`] and [`build`](Self::build) returns it — nothing is
/// sent, and nothing panics (a panic in a wasm32 consumer is an
/// unrecoverable trap).
///
/// ```
/// use plexus_client::Params;
///
/// let params = Params::new()
///     .param("id", "MNG-2")
///     .optional("epic", None::<String>)
///     .build()
///     .unwrap();
/// assert_eq!(params, serde_json::json!({"id": "MNG-2"}));
/// ```
#[derive(Debug, Default)]
pub struct Params {
    map: Map<String, Value>,
    error: Option<ClientError>,
}

impl Params {
    /// An empty params object — what a method with no parameters sends.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add `name` with `value`, always.
    pub fn param<V: Serialize>(mut self, name: &'static str, value: V) -> Self {
        if self.error.is_none() {
            match serde_json::to_value(value) {
                Ok(json) => {
                    self.map.insert(name.to_string(), json);
                }
                Err(e) => {
                    self.error = Some(ClientError::Encode {
                        param: name,
                        message: e.to_string(),
                    })
                }
            }
        }
        self
    }

    /// Add `name` when `value` is `Some`; omit it when `None`.
    pub fn optional<V: Serialize>(self, name: &'static str, value: Option<V>) -> Self {
        match value {
            Some(value) => self.param(name, value),
            None => self,
        }
    }

    /// The params object, or the first argument that would not encode.
    pub fn build(self) -> Result<Value, ClientError> {
        match self.error {
            Some(e) => Err(e),
            None => Ok(Value::Object(self.map)),
        }
    }
}
