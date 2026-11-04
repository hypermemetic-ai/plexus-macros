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

use serde::de::Deserializer;

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
    // Not yet: every read is serde's, buffer and all.
    T::tagged(deserializer)
}
