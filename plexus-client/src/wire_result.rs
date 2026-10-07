//! The wire encoding of a Result item: `Ok(t)` ⇄ `{"ok": t}`,
//! `Err(e)` ⇄ `{"err": e}`.
//!
//! A method that returns `Result<T, E>` (or streams `Result` items) sends
//! each item as data — an err is the activation saying no, not the call
//! failing — and this is its shape. Use it as a serde `with` module:
//!
//! ```
//! use plexus_client::wire_result;
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Debug, PartialEq, Serialize, Deserialize)]
//! struct Wrapped(#[serde(with = "wire_result")] Result<u32, String>);
//!
//! let ok = serde_json::to_value(Wrapped(Ok(7))).unwrap();
//! assert_eq!(ok, serde_json::json!({"ok": 7}));
//!
//! let err: Wrapped = serde_json::from_value(serde_json::json!({"err": "no"})).unwrap();
//! assert_eq!(err, Wrapped(Err("no".into())));
//! ```
//!
//! or call [`serialize`] / [`deserialize`] directly with any serde
//! (de)serializer — a `serde_json::Value` is one.
//!
//! Decoding accepts exactly an object with one key, `ok` or `err`; anything
//! else (another key, both keys, no key, a non-object) is an error whose
//! message names both keys.
//!
//! Works inside adjacently-tagged enums
//! (`#[serde(tag = "method", content = "event")]`) on a newtype variant —
//! the shape of the derived `{Ns}Event` aggregate.

use std::fmt;
use std::marker::PhantomData;

use serde::de::{self, IgnoredAny, MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

const OK: &str = "ok";
const ERR: &str = "err";
const EXPECTED: &str = "an object with exactly one key, `ok` or `err`";

/// `Ok(t)` → `{"ok": t}`, `Err(e)` → `{"err": e}`.
pub fn serialize<T, E, S>(value: &Result<T, E>, serializer: S) -> Result<S::Ok, S::Error>
where
    T: Serialize,
    E: Serialize,
    S: Serializer,
{
    let mut map = serializer.serialize_map(Some(1))?;
    match value {
        Ok(t) => map.serialize_entry(OK, t)?,
        Err(e) => map.serialize_entry(ERR, e)?,
    }
    map.end()
}

/// `{"ok": t}` → `Ok(t)`, `{"err": e}` → `Err(e)`; any other shape is an
/// error naming both keys.
pub fn deserialize<'de, T, E, D>(deserializer: D) -> Result<Result<T, E>, D::Error>
where
    T: Deserialize<'de>,
    E: Deserialize<'de>,
    D: Deserializer<'de>,
{
    deserializer.deserialize_map(ResultVisitor(PhantomData))
}

struct ResultVisitor<T, E>(PhantomData<fn() -> (T, E)>);

impl<'de, T, E> Visitor<'de> for ResultVisitor<T, E>
where
    T: Deserialize<'de>,
    E: Deserialize<'de>,
{
    type Value = Result<T, E>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(EXPECTED)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let value = match map.next_key::<Key>()? {
            Some(Key::Ok) => Ok(map.next_value::<T>()?),
            Some(Key::Err) => Err(map.next_value::<E>()?),
            None => {
                return Err(de::Error::custom(format_args!(
                    "expected {EXPECTED}, found an empty object"
                )))
            }
        };
        if map.next_key::<IgnoredAny>()?.is_some() {
            return Err(de::Error::custom(format_args!(
                "expected {EXPECTED}, found more than one key"
            )));
        }
        Ok(value)
    }
}

/// The one key of a Result item.
enum Key {
    Ok,
    Err,
}

impl<'de> Deserialize<'de> for Key {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_identifier(KeyVisitor)
    }
}

struct KeyVisitor;

impl<'de> Visitor<'de> for KeyVisitor {
    type Value = Key;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the key `ok` or `err`")
    }

    fn visit_str<X: de::Error>(self, key: &str) -> Result<Key, X> {
        match key {
            OK => Ok(Key::Ok),
            ERR => Ok(Key::Err),
            other => Err(X::custom(format_args!(
                "expected {EXPECTED}, found key `{other}`"
            ))),
        }
    }

    fn visit_bytes<X: de::Error>(self, key: &[u8]) -> Result<Key, X> {
        match key {
            b"ok" => Ok(Key::Ok),
            b"err" => Ok(Key::Err),
            other => Err(X::custom(format_args!(
                "expected {EXPECTED}, found key `{}`",
                String::from_utf8_lossy(other)
            ))),
        }
    }
}
