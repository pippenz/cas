//! Custom deserializers for flexible numeric type coercion.
//!
//! Some MCP client implementations (including Claude Code) serialize numeric
//! parameters as JSON strings (e.g., `"3"` instead of `3`). These helpers
//! accept both native JSON numbers and string-encoded numbers so tool calls
//! are not rejected due to type mismatch.

use serde::{de, Deserializer};
use std::fmt;

/// Generate an `Option<$target>` deserializer that accepts numbers, strings, and null.
///
/// Produces a public function `$fn_name` usable with `#[serde(deserialize_with = "...")]`.
macro_rules! option_numeric_deser {
    ($fn_name:ident, $target:ty, $desc:expr) => {
        pub fn $fn_name<'de, D>(deserializer: D) -> Result<Option<$target>, D::Error>
        where
            D: Deserializer<'de>,
        {
            struct V;
            impl<'de> de::Visitor<'de> for V {
                type Value = Option<$target>;

                fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                    write!(f, "{}, a string containing one, or null", $desc)
                }

                fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                    Ok(None)
                }
                fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                    Ok(None)
                }
                fn visit_some<D2: Deserializer<'de>>(
                    self,
                    d: D2,
                ) -> Result<Self::Value, D2::Error> {
                    d.deserialize_any(self)
                }
                fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                    <$target>::try_from(v).map(Some).map_err(|_| {
                        de::Error::invalid_value(
                            de::Unexpected::Unsigned(v),
                            &concat!("a ", stringify!($target), " value"),
                        )
                    })
                }
                fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                    <$target>::try_from(v).map(Some).map_err(|_| {
                        de::Error::invalid_value(
                            de::Unexpected::Signed(v),
                            &concat!("a ", stringify!($target), " value"),
                        )
                    })
                }
                fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                    let t = v.trim();
                    if t.is_empty() {
                        return Ok(None);
                    }
                    t.parse::<$target>().map(Some).map_err(|_| {
                        de::Error::invalid_value(
                            de::Unexpected::Str(v),
                            &concat!("a string encoding a ", stringify!($target)),
                        )
                    })
                }
            }

            deserializer.deserialize_option(V)
        }
    };
}

option_numeric_deser!(option_i32, i32, "an i32 integer");
option_numeric_deser!(option_i64, i64, "an i64 integer");
option_numeric_deser!(option_u32, u32, "a u32 integer");
option_numeric_deser!(option_usize, usize, "a usize integer");
option_numeric_deser!(option_u64, u64, "a u64 integer");

/// Priority deserializer that accepts numeric 0-4, string-encoded numerics,
/// and the named aliases workers and supervisors reach for naturally
/// (`critical`, `high`, `medium`, `low`, `backlog`). On failure the error
/// message lists the valid numeric range AND the named aliases so the caller
/// doesn't have to guess.
///
/// Addresses cas-ca63: numeric-enum-as-string is a universal failure pattern
/// across CAS callers, and the previous `option_u8` simply reported
/// "expected a string encoding a u8" with no hint at what was valid.
pub fn option_priority<'de, D>(deserializer: D) -> Result<Option<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    const VALID_ALIASES: &str =
        "0=critical, 1=high, 2=medium, 3=low, 4=backlog (numeric 0-4 or named alias)";

    struct V;
    impl<'de> de::Visitor<'de> for V {
        type Value = Option<u8>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(
                f,
                "priority as integer 0-4, numeric string \"0\"-\"4\", or named alias \
                 (critical, high, medium, low, backlog)"
            )
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
            if v > 4 {
                return Err(de::Error::invalid_value(
                    de::Unexpected::Unsigned(v),
                    &VALID_ALIASES,
                ));
            }
            Ok(Some(v as u8))
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
            if !(0..=4).contains(&v) {
                return Err(de::Error::invalid_value(
                    de::Unexpected::Signed(v),
                    &VALID_ALIASES,
                ));
            }
            Ok(Some(v as u8))
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            let t = v.trim();
            if t.is_empty() {
                return Ok(None);
            }
            // Named aliases first.
            match t.to_ascii_lowercase().as_str() {
                "critical" | "crit" | "p0" => return Ok(Some(0)),
                "high" | "p1" => return Ok(Some(1)),
                "medium" | "med" | "normal" | "p2" => return Ok(Some(2)),
                "low" | "p3" => return Ok(Some(3)),
                "backlog" | "later" | "p4" => return Ok(Some(4)),
                _ => {}
            }
            // Fall back to numeric string.
            match t.parse::<u8>() {
                Ok(n) if n <= 4 => Ok(Some(n)),
                _ => Err(de::Error::invalid_value(
                    de::Unexpected::Str(v),
                    &VALID_ALIASES,
                )),
            }
        }
    }

    deserializer.deserialize_option(V)
}

/// Flexible boolean deserializer that accepts native bool, string "true"/"false"
/// (any case), and numeric 0/1. Fixes the "string \"true\", expected a boolean"
/// rejections observed in the 2026-04-09 log audit (cas-ca63 Issue 2).
pub fn option_bool<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
where
    D: Deserializer<'de>,
{
    const EXPECT: &str = "boolean, \"true\"/\"false\" (any case), or 0/1";

    struct V;
    impl<'de> de::Visitor<'de> for V {
        type Value = Option<bool>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(f, "{EXPECT}")
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
        fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
            Ok(Some(v))
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
            match v {
                0 => Ok(Some(false)),
                1 => Ok(Some(true)),
                _ => Err(de::Error::invalid_value(
                    de::Unexpected::Unsigned(v),
                    &EXPECT,
                )),
            }
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
            match v {
                0 => Ok(Some(false)),
                1 => Ok(Some(true)),
                _ => Err(de::Error::invalid_value(
                    de::Unexpected::Signed(v),
                    &EXPECT,
                )),
            }
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            match v.trim().to_ascii_lowercase().as_str() {
                "" => Ok(None),
                "true" | "yes" | "1" | "on" => Ok(Some(true)),
                "false" | "no" | "0" | "off" => Ok(Some(false)),
                _ => Err(de::Error::invalid_value(de::Unexpected::Str(v), &EXPECT)),
            }
        }
    }

    deserializer.deserialize_option(V)
}
