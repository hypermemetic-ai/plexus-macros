//! PERF.115-43 · an internally tagged enum read with its tag first: the
//! runtime of `#[derive(plexus_macros::Deserialize)]`.
//!
//! serde reads an internally tagged enum, `#[serde(tag = "type")]` (every
//! house event), whole into a private buffer before it reads the tag, then
//! reads the variant out of the buffer: it cannot know the variant until it
//! has read `type`, and `type` may come anywhere. The house writes `type`
//! first. So the derive's `Deserialize` reads the first key: when it is the
//! tag, the tag's value names the variant and the rest of the object is read
//! straight into it; when it is not, the object goes to serde's own read,
//! buffer and all, as before.
//!
//! The derive has serde derive the enum twice more, each with
//! `#[serde(remote = "…")]`, so that each builds the enum itself:
//!
//! - [`TypeFirst::tagged`], the enum as written: serde's buffered read, for
//!   an object whose tag does not come first, a sequence, anything else, and
//!   a variant the rest of an object cannot be read straight into;
//! - [`TypeFirst::named`], the enum without its tag: serde's read of each
//!   variant by name, handed here an enum access whose variant is the tag's
//!   value and whose content is the rest of the object.
//!
//! Read straight: a struct variant, its fields from the object itself, and a
//! unit variant, the rest of the object read and dropped as serde drops it.
//! A newtype variant, a struct variant with a flattened field and a variant
//! read `with` a function go to serde's read: the object is handed to it
//! again, its tag and the tag's value first, then the rest, still unread.
//!
//! What serde's read refuses, this refuses, in serde's words: a second tag
//! wherever it comes, an unknown variant, a missing or mistyped field, an
//! unknown field under `deny_unknown_fields`. A value is read by the
//! deserializer itself rather than out of serde's buffer, so a value only
//! the input can carry (a `RawValue`, a map keyed by numbers) now reaches a
//! variant whose tag comes first, and a fault inside a field's value is named
//! as the deserializer names it, the way it is for every type that is not an
//! internally tagged enum.

use std::fmt;
use std::marker::PhantomData;

use serde::de::value::{MapAccessDeserializer, SeqAccessDeserializer};
use serde::de::{
    self, DeserializeSeed, Deserializer, EnumAccess, IgnoredAny, MapAccess, SeqAccess, Unexpected, VariantAccess,
    Visitor,
};

/// An internally tagged enum read with its tag first: what
/// `#[derive(plexus_macros::Deserialize)]` implements, beside the enum's
/// `Deserialize`, which is [`deserialize`].
pub trait TypeFirst<'de>: Sized {
    /// The tag's name, `#[serde(tag = "…")]`.
    const TAG: &'static str;

    /// What serde says it expected when the input is neither an object nor a
    /// sequence: `internally tagged enum Name`, or the enum's own
    /// `#[serde(expecting = "…")]`.
    const EXPECTING: &'static str;

    /// serde's read of the enum as written, buffer and all.
    fn tagged<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error>;

    /// serde's read of the enum's variants by name, as an externally tagged
    /// enum's: `deserialize_enum`, its variant the tag's value.
    fn named<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error>;
}

/// `T`'s `Deserialize`: its tag first, else serde's read.
pub fn deserialize<'de, T, D>(deserializer: D) -> Result<T, D::Error>
where
    T: TypeFirst<'de>,
    D: Deserializer<'de>,
{
    deserializer.deserialize_any(Object(PhantomData))
}

/// The input as serde's read of an internally tagged enum takes it, through
/// `deserialize_any`: an object, or a sequence whose first element is the
/// tag. Anything else is refused as serde refuses it, with its `expecting`.
struct Object<T>(PhantomData<T>);

impl<'de, T: TypeFirst<'de>> Visitor<'de> for Object<T> {
    type Value = T;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(T::EXPECTING)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<T, A::Error> {
        match map.next_key_seed(KeySeed(T::TAG))? {
            Some(Key::Tag) => {}
            // The first key is not the tag, or there is none: serde's read,
            // that key handed back to it first.
            Some(Key::Other(key)) => return T::tagged(MapAccessDeserializer::new(Replay::new(Some(key), None, map))),
            None => return T::tagged(MapAccessDeserializer::new(Replay::new(None, None, map))),
        }
        let tag = map.next_value_seed(TagValue)?;
        let mut deferred = false;
        let read = T::named(Named { map: &mut map, tag: T::TAG, value: &tag, deferred: &mut deferred });
        if !deferred {
            return read;
        }
        // A variant the rest of the object cannot be read straight into:
        // serde's read, the tag and its value handed back first, the rest
        // still unread.
        T::tagged(MapAccessDeserializer::new(Replay::new(Some(Scalar::Str(T::TAG)), Some(tag), map)))
    }

    fn visit_seq<S: SeqAccess<'de>>(self, seq: S) -> Result<T, S::Error> {
        T::tagged(SeqAccessDeserializer::new(seq))
    }
}

/// The rest of an object after its tag, as serde's read of the variants by
/// name takes it: an enum whose variant is the tag's value and whose content
/// is the object's other keys.
struct Named<'a, 'de, A> {
    map: &'a mut A,
    tag: &'static str,
    value: &'a Scalar<'de>,
    /// Set when the variant is one serde reads through its buffer.
    deferred: &'a mut bool,
}

impl<'a, 'de, A: MapAccess<'de>> Named<'a, 'de, A> {
    /// The variant is read by serde's read of the whole object instead: this
    /// error ends the read by name, and is never seen.
    fn defer<T>(self) -> Result<T, A::Error> {
        *self.deferred = true;
        Err(de::Error::custom("a variant read through serde's buffer"))
    }
}

impl<'a, 'de, A: MapAccess<'de>> Deserializer<'de> for Named<'a, 'de, A> {
    type Error = A::Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, A::Error> {
        visitor.visit_enum(self)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct enum identifier ignored_any
    }
}

impl<'a, 'de, A: MapAccess<'de>> EnumAccess<'de> for Named<'a, 'de, A> {
    type Error = A::Error;
    type Variant = Self;

    fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, Self), A::Error> {
        let variant = seed.deserialize(ScalarRef(self.value, PhantomData::<A::Error>))?;
        Ok((variant, self))
    }
}

impl<'a, 'de, A: MapAccess<'de>> VariantAccess<'de> for Named<'a, 'de, A> {
    type Error = A::Error;

    /// serde reads a unit variant's object to its end and drops every other
    /// key; a second tag it refuses as it reads.
    fn unit_variant(self) -> Result<(), A::Error> {
        loop {
            match self.map.next_key_seed(KeySeed(self.tag))? {
                None => return Ok(()),
                Some(Key::Tag) => return Err(de::Error::duplicate_field(self.tag)),
                Some(Key::Other(_)) => {
                    self.map.next_value::<IgnoredAny>()?;
                }
            }
        }
    }

    /// A struct variant: its fields read straight from the object.
    fn struct_variant<V: Visitor<'de>>(
        self,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, A::Error> {
        visitor.visit_map(Rest { map: self.map, tag: self.tag })
    }

    /// A newtype variant, and what serde reads as one: a struct variant with
    /// a flattened field, a variant read `with` a function. serde's read.
    fn newtype_variant_seed<S: DeserializeSeed<'de>>(self, _seed: S) -> Result<S::Value, A::Error> {
        self.defer()
    }

    /// No internally tagged enum has one (serde refuses it as it derives);
    /// serde's read says so.
    fn tuple_variant<V: Visitor<'de>>(self, _len: usize, _visitor: V) -> Result<V::Value, A::Error> {
        self.defer()
    }
}

/// A struct variant's object after its tag: its other keys, handed to the
/// variant as serde's buffer hands them, and a second tag refused where
/// serde refuses it.
struct Rest<'a, A> {
    map: &'a mut A,
    tag: &'static str,
}

impl<'a, 'de, A: MapAccess<'de>> MapAccess<'de> for Rest<'a, A> {
    type Error = A::Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, A::Error> {
        match self.map.next_key_seed(KeySeed(self.tag))? {
            None => Ok(None),
            Some(Key::Tag) => Err(de::Error::duplicate_field(self.tag)),
            Some(Key::Other(key)) => seed.deserialize(ScalarDe(key, PhantomData)).map(Some),
        }
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, A::Error> {
        self.map.next_value_seed(seed)
    }

    fn size_hint(&self) -> Option<usize> {
        self.map.size_hint()
    }
}

/// An object handed to serde's read with what was already read of it put
/// back in front: its first key (and that key's value, when it was read).
struct Replay<'de, A> {
    key: Option<Scalar<'de>>,
    value: Option<Scalar<'de>>,
    map: A,
}

impl<'de, A> Replay<'de, A> {
    fn new(key: Option<Scalar<'de>>, value: Option<Scalar<'de>>, map: A) -> Self {
        Replay { key, value, map }
    }
}

impl<'de, A: MapAccess<'de>> MapAccess<'de> for Replay<'de, A> {
    type Error = A::Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, A::Error> {
        match self.key.take() {
            Some(key) => seed.deserialize(ScalarDe(key, PhantomData)).map(Some),
            None => self.map.next_key_seed(seed),
        }
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, A::Error> {
        match self.value.take() {
            Some(value) => seed.deserialize(ScalarDe(value, PhantomData)),
            None => self.map.next_value_seed(seed),
        }
    }

    fn size_hint(&self) -> Option<usize> {
        self.map.size_hint().map(|n| n + usize::from(self.key.is_some()))
    }
}

/// A key of an object, read as serde's buffer reads one
/// (`deserialize_any`): the tag, or any other key, kept to be handed on.
#[derive(Clone, Copy)]
struct KeySeed(&'static str);

enum Key<'de> {
    Tag,
    Other(Scalar<'de>),
}

impl<'de> DeserializeSeed<'de> for KeySeed {
    type Value = Key<'de>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Key<'de>, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for KeySeed {
    type Value = Key<'de>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "a type tag `{}` or any other value", self.0)
    }

    fn visit_borrowed_str<E: de::Error>(self, v: &'de str) -> Result<Key<'de>, E> {
        Ok(if v == self.0 { Key::Tag } else { Key::Other(Scalar::Str(v)) })
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Key<'de>, E> {
        Ok(if v == self.0 { Key::Tag } else { Key::Other(Scalar::String(v.to_owned())) })
    }

    fn visit_string<E: de::Error>(self, v: String) -> Result<Key<'de>, E> {
        Ok(if v == self.0 { Key::Tag } else { Key::Other(Scalar::String(v)) })
    }

    fn visit_borrowed_bytes<E: de::Error>(self, v: &'de [u8]) -> Result<Key<'de>, E> {
        Ok(if v == self.0.as_bytes() { Key::Tag } else { Key::Other(Scalar::Bytes(v)) })
    }

    fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<Key<'de>, E> {
        Ok(if v == self.0.as_bytes() { Key::Tag } else { Key::Other(Scalar::ByteBuf(v.to_vec())) })
    }

    fn visit_byte_buf<E: de::Error>(self, v: Vec<u8>) -> Result<Key<'de>, E> {
        Ok(if v == self.0.as_bytes() { Key::Tag } else { Key::Other(Scalar::ByteBuf(v)) })
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Key<'de>, E> {
        Ok(Key::Other(Scalar::U64(v)))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Key<'de>, E> {
        Ok(Key::Other(Scalar::I64(v)))
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Key<'de>, E> {
        Ok(Key::Other(Scalar::F64(v)))
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Key<'de>, E> {
        Ok(Key::Other(Scalar::Bool(v)))
    }

    fn visit_char<E: de::Error>(self, v: char) -> Result<Key<'de>, E> {
        Ok(Key::Other(Scalar::Char(v)))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Key<'de>, E> {
        Ok(Key::Other(Scalar::Unit))
    }
}

/// The tag's value, read as serde reads it (`deserialize_identifier`, with
/// a variant identifier's `expecting`), kept to be handed to the variant's
/// name and, if need be, to serde's read.
struct TagValue;

impl<'de> DeserializeSeed<'de> for TagValue {
    type Value = Scalar<'de>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Scalar<'de>, D::Error> {
        deserializer.deserialize_identifier(self)
    }
}

impl<'de> Visitor<'de> for TagValue {
    type Value = Scalar<'de>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("variant identifier")
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Scalar<'de>, E> {
        Ok(Scalar::U64(v))
    }

    fn visit_borrowed_str<E: de::Error>(self, v: &'de str) -> Result<Scalar<'de>, E> {
        Ok(Scalar::Str(v))
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Scalar<'de>, E> {
        Ok(Scalar::String(v.to_owned()))
    }

    fn visit_string<E: de::Error>(self, v: String) -> Result<Scalar<'de>, E> {
        Ok(Scalar::String(v))
    }

    fn visit_borrowed_bytes<E: de::Error>(self, v: &'de [u8]) -> Result<Scalar<'de>, E> {
        Ok(Scalar::Bytes(v))
    }

    fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<Scalar<'de>, E> {
        Ok(Scalar::ByteBuf(v.to_vec()))
    }

    fn visit_byte_buf<E: de::Error>(self, v: Vec<u8>) -> Result<Scalar<'de>, E> {
        Ok(Scalar::ByteBuf(v))
    }
}

/// A key, or the tag's value, as it was read: what serde's buffer keeps of
/// it, to be visited again as it was first visited.
enum Scalar<'de> {
    Str(&'de str),
    String(String),
    Bytes(&'de [u8]),
    ByteBuf(Vec<u8>),
    U64(u64),
    I64(i64),
    F64(f64),
    Bool(bool),
    Char(char),
    Unit,
}

impl Scalar<'_> {
    fn unexpected(&self) -> Unexpected<'_> {
        match self {
            Scalar::Str(v) => Unexpected::Str(v),
            Scalar::String(v) => Unexpected::Str(v),
            Scalar::Bytes(v) => Unexpected::Bytes(v),
            Scalar::ByteBuf(v) => Unexpected::Bytes(v),
            Scalar::U64(v) => Unexpected::Unsigned(*v),
            Scalar::I64(v) => Unexpected::Signed(*v),
            Scalar::F64(v) => Unexpected::Float(*v),
            Scalar::Bool(v) => Unexpected::Bool(*v),
            Scalar::Char(v) => Unexpected::Char(*v),
            Scalar::Unit => Unexpected::Unit,
        }
    }
}

/// A [`Scalar`] visited again: `deserialize_any` as it was first visited,
/// `deserialize_identifier` as serde's buffer visits what it kept (a string,
/// bytes or a number name a field or a variant; anything else is refused).
struct ScalarDe<'de, E>(Scalar<'de>, PhantomData<E>);

impl<'de, E: de::Error> Deserializer<'de> for ScalarDe<'de, E> {
    type Error = E;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, E> {
        match self.0 {
            Scalar::Str(v) => visitor.visit_borrowed_str(v),
            Scalar::String(v) => visitor.visit_string(v),
            Scalar::Bytes(v) => visitor.visit_borrowed_bytes(v),
            Scalar::ByteBuf(v) => visitor.visit_byte_buf(v),
            Scalar::U64(v) => visitor.visit_u64(v),
            Scalar::I64(v) => visitor.visit_i64(v),
            Scalar::F64(v) => visitor.visit_f64(v),
            Scalar::Bool(v) => visitor.visit_bool(v),
            Scalar::Char(v) => visitor.visit_char(v),
            Scalar::Unit => visitor.visit_unit(),
        }
    }

    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, E> {
        match self.0 {
            Scalar::Str(v) => visitor.visit_borrowed_str(v),
            Scalar::String(v) => visitor.visit_string(v),
            Scalar::Bytes(v) => visitor.visit_borrowed_bytes(v),
            Scalar::ByteBuf(v) => visitor.visit_byte_buf(v),
            Scalar::U64(v) => visitor.visit_u64(v),
            ref other => Err(de::Error::invalid_type(other.unexpected(), &visitor)),
        }
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct enum ignored_any
    }
}

/// The tag's value visited again by reference, for the variant's name: it is
/// kept whole for serde's read, which may need it after.
struct ScalarRef<'a, 'de, E>(&'a Scalar<'de>, PhantomData<E>);

impl<'de, E: de::Error> Deserializer<'de> for ScalarRef<'_, 'de, E> {
    type Error = E;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, E> {
        self.deserialize_identifier(visitor)
    }

    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, E> {
        match *self.0 {
            Scalar::Str(v) => visitor.visit_borrowed_str(v),
            Scalar::String(ref v) => visitor.visit_str(v),
            Scalar::Bytes(v) => visitor.visit_borrowed_bytes(v),
            Scalar::ByteBuf(ref v) => visitor.visit_bytes(v),
            Scalar::U64(v) => visitor.visit_u64(v),
            ref other => Err(de::Error::invalid_type(other.unexpected(), &visitor)),
        }
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct enum ignored_any
    }
}
