//! Custom deserializers for flexible numeric type coercion.
//!
//! Some MCP client implementations (including Claude Code) serialize numeric
//! parameters as JSON strings (e.g., `"3"` instead of `3`). These helpers
//! accept both native JSON numbers and string-encoded numbers so tool calls
//! are not rejected due to type mismatch.

use serde::{de, Deserializer};
use std::fmt;

/// Deserialize `Option<u8>` accepting both numeric and string-encoded values.
///
/// Handles: `null` → `None`, absent field (via `#[serde(default)]`) → `None`,
/// `3` → `Some(3)`, `"3"` → `Some(3)`.
pub fn option_u8<'de, D>(deserializer: D) -> Result<Option<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    struct V;
    impl<'de> de::Visitor<'de> for V {
        type Value = Option<u8>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("an integer 0-255, a string containing an integer, or null")
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_some<D2: Deserializer<'de>>(self, d: D2) -> Result<Self::Value, D2::Error> {
            d.deserialize_any(self)
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
            u8::try_from(v).map(Some).map_err(|_| {
                de::Error::invalid_value(de::Unexpected::Unsigned(v), &"a u8 value (0-255)")
            })
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
            u8::try_from(v).map(Some).map_err(|_| {
                de::Error::invalid_value(de::Unexpected::Signed(v), &"a u8 value (0-255)")
            })
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            let t = v.trim();
            if t.is_empty() {
                return Ok(None);
            }
            t.parse::<u8>().map(Some).map_err(|_| {
                de::Error::invalid_value(de::Unexpected::Str(v), &"a string encoding a u8 (0-255)")
            })
        }
    }

    deserializer.deserialize_option(V)
}

/// Deserialize `Option<i32>` accepting both numeric and string-encoded values.
///
/// Handles: `null` → `None`, absent field (via `#[serde(default)]`) → `None`,
/// `3` → `Some(3)`, `"3"` → `Some(3)`.
pub fn option_i32<'de, D>(deserializer: D) -> Result<Option<i32>, D::Error>
where
    D: Deserializer<'de>,
{
    struct V;
    impl<'de> de::Visitor<'de> for V {
        type Value = Option<i32>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("an integer, a string containing an integer, or null")
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_some<D2: Deserializer<'de>>(self, d: D2) -> Result<Self::Value, D2::Error> {
            d.deserialize_any(self)
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
            i32::try_from(v).map(Some).map_err(|_| {
                de::Error::invalid_value(de::Unexpected::Unsigned(v), &"an i32 value")
            })
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
            i32::try_from(v).map(Some).map_err(|_| {
                de::Error::invalid_value(de::Unexpected::Signed(v), &"an i32 value")
            })
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            let t = v.trim();
            if t.is_empty() {
                return Ok(None);
            }
            t.parse::<i32>().map(Some).map_err(|_| {
                de::Error::invalid_value(de::Unexpected::Str(v), &"a string encoding an i32")
            })
        }
    }

    deserializer.deserialize_option(V)
}
