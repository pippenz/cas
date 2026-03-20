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

option_numeric_deser!(option_u8, u8, "an integer 0-255");
option_numeric_deser!(option_i32, i32, "an i32 integer");
option_numeric_deser!(option_i64, i64, "an i64 integer");
option_numeric_deser!(option_u32, u32, "a u32 integer");
option_numeric_deser!(option_usize, usize, "a usize integer");
option_numeric_deser!(option_u64, u64, "a u64 integer");
