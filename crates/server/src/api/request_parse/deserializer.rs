//! Deserializers that read a request already split into JSON values, or into named strings, as
//! a form, a query string and path parameters are. Their error is a [`Fault`]: the kind of
//! fault serde reports, as data, with none of serde's message text, which can quote what was
//! sent.

use serde::de::value::{MapAccessDeserializer, MapDeserializer, SeqDeserializer};
use serde::de::{self, Deserializer, IntoDeserializer, Visitor};
use serde::forward_to_deserialize_any;
use serde_json::Value;

/// What serde found wrong with a request, by the kind of fault alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Fault {
    /// A field the type needs was not sent.
    Missing(&'static str),
    /// A field, named as it was sent, that the type does not take.
    Unknown(String),
    /// A field that was sent more than once.
    Duplicate(&'static str),
    /// A value of the wrong type, of a value out of range, or any other fault.
    Invalid,
}

/// A JSON value read as a request through a [`Fault`] error.
#[derive(Debug)]
pub(super) struct JsonValue(pub(super) Value);

/// One value of a form, a query string or path parameters, read from its text.
#[derive(Debug)]
pub(super) struct TextValue(pub(super) String);

/// The pairs of a form or a query string, read as a map of names to [`TextValue`]s.
#[derive(Debug)]
pub(super) struct TextPairs(pub(super) Vec<(String, String)>);

/// A route's path parameters: a map, or a list in the route's order, of names to
/// [`TextValue`]s, or a lone value when the route has one.
#[derive(Debug)]
pub(super) struct PathValues(pub(super) Vec<(String, String)>);

impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the request does not parse")
    }
}

impl std::error::Error for Fault {}

impl de::Error for Fault {
    fn custom<T: std::fmt::Display>(_message: T) -> Self {
        Self::Invalid
    }

    fn missing_field(field: &'static str) -> Self {
        Self::Missing(field)
    }

    fn unknown_field(field: &str, _expected: &'static [&'static str]) -> Self {
        Self::Unknown(field.to_string())
    }

    fn duplicate_field(field: &'static str) -> Self {
        Self::Duplicate(field)
    }
}

/// Visit `entries` as a map of names to values, refusing any left unread.
fn visit_map<'de, V, I, K, T>(visitor: V, entries: I) -> Result<V::Value, Fault>
where
    V: Visitor<'de>,
    I: Iterator<Item = (K, T)>,
    K: IntoDeserializer<'de, Fault>,
    T: IntoDeserializer<'de, Fault>,
{
    let mut map = MapDeserializer::new(entries);
    let value = visitor.visit_map(&mut map)?;
    map.end()?;
    Ok(value)
}

/// Visit `items` as a list, refusing any left unread.
fn visit_seq<'de, V, I, T>(visitor: V, items: I) -> Result<V::Value, Fault>
where
    V: Visitor<'de>,
    I: Iterator<Item = T>,
    T: IntoDeserializer<'de, Fault>,
{
    let mut seq = SeqDeserializer::new(items);
    let value = visitor.visit_seq(&mut seq)?;
    seq.end()?;
    Ok(value)
}

impl IntoDeserializer<'_, Fault> for JsonValue {
    type Deserializer = Self;

    fn into_deserializer(self) -> Self {
        self
    }
}

impl<'de> Deserializer<'de> for JsonValue {
    type Error = Fault;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Fault> {
        match self.0 {
            Value::Null => visitor.visit_unit(),
            Value::Bool(value) => visitor.visit_bool(value),
            Value::Number(number) => {
                if let Some(value) = number.as_u64() {
                    visitor.visit_u64(value)
                } else if let Some(value) = number.as_i64() {
                    visitor.visit_i64(value)
                } else {
                    number
                        .as_f64()
                        .map_or(Err(Fault::Invalid), |value| visitor.visit_f64(value))
                }
            }
            Value::String(value) => visitor.visit_string(value),
            Value::Array(items) => visit_seq(visitor, items.into_iter().map(Self)),
            Value::Object(entries) => visit_map(
                visitor,
                entries.into_iter().map(|(key, value)| (key, Self(value))),
            ),
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Fault> {
        match self.0 {
            Value::Null => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Fault> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Fault> {
        match self.0 {
            Value::String(variant) => visitor.visit_enum(variant.into_deserializer()),
            Value::Object(entries) if entries.len() == 1 => {
                visitor.visit_enum(MapAccessDeserializer::new(MapDeserializer::new(
                    entries.into_iter().map(|(key, value)| (key, Self(value))),
                )))
            }
            _ => Err(Fault::Invalid),
        }
    }

    forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf
        unit unit_struct seq tuple tuple_struct map struct identifier ignored_any
    }
}

/// Deserializer methods of [`TextValue`] that parse its text as a number or a bool.
macro_rules! parse_text {
    ($($method:ident => $visit:ident: $kind:ty),* $(,)?) => {
        $(
            fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Fault> {
                visitor.$visit(self.0.parse::<$kind>().map_err(|_| Fault::Invalid)?)
            }
        )*
    };
}

impl IntoDeserializer<'_, Fault> for TextValue {
    type Deserializer = Self;

    fn into_deserializer(self) -> Self {
        self
    }
}

impl<'de> Deserializer<'de> for TextValue {
    type Error = Fault;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Fault> {
        visitor.visit_string(self.0)
    }

    parse_text! {
        deserialize_bool => visit_bool: bool,
        deserialize_i8 => visit_i8: i8,
        deserialize_i16 => visit_i16: i16,
        deserialize_i32 => visit_i32: i32,
        deserialize_i64 => visit_i64: i64,
        deserialize_i128 => visit_i128: i128,
        deserialize_u8 => visit_u8: u8,
        deserialize_u16 => visit_u16: u16,
        deserialize_u32 => visit_u32: u32,
        deserialize_u64 => visit_u64: u64,
        deserialize_u128 => visit_u128: u128,
        deserialize_f32 => visit_f32: f32,
        deserialize_f64 => visit_f64: f64,
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Fault> {
        visitor.visit_some(self)
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Fault> {
        visitor.visit_unit()
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Fault> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Fault> {
        visitor.visit_enum(self.0.into_deserializer())
    }

    forward_to_deserialize_any! {
        char str string bytes byte_buf unit_struct seq tuple tuple_struct map struct identifier
        ignored_any
    }
}

impl<'de> Deserializer<'de> for TextPairs {
    type Error = Fault;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Fault> {
        visit_map(
            visitor,
            self.0
                .into_iter()
                .map(|(name, value)| (name, TextValue(value))),
        )
    }

    forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf
        option unit unit_struct newtype_struct seq tuple tuple_struct map struct enum identifier
        ignored_any
    }
}

impl PathValues {
    /// The route's one value, for a type that is a single value.
    fn single(self) -> Result<TextValue, Fault> {
        let mut values = self.0.into_iter();
        match (values.next(), values.next()) {
            (Some((_, value)), None) => Ok(TextValue(value)),
            _ => Err(Fault::Invalid),
        }
    }
}

/// Deserializer methods of [`PathValues`] that read the route's one value.
macro_rules! single_value {
    ($($method:ident),* $(,)?) => {
        $(
            fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Fault> {
                self.single()?.$method(visitor)
            }
        )*
    };
}

impl<'de> Deserializer<'de> for PathValues {
    type Error = Fault;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Fault> {
        self.deserialize_map(visitor)
    }

    single_value! {
        deserialize_bool, deserialize_i8, deserialize_i16, deserialize_i32, deserialize_i64,
        deserialize_i128, deserialize_u8, deserialize_u16, deserialize_u32, deserialize_u64,
        deserialize_u128, deserialize_f32, deserialize_f64, deserialize_char, deserialize_str,
        deserialize_string, deserialize_bytes, deserialize_byte_buf, deserialize_option,
        deserialize_unit, deserialize_identifier,
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Fault> {
        visitor.visit_unit()
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Fault> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Fault> {
        visit_seq(
            visitor,
            self.0.into_iter().map(|(_, value)| TextValue(value)),
        )
    }

    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, Fault> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, Fault> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Fault> {
        visit_map(
            visitor,
            self.0
                .into_iter()
                .map(|(name, value)| (name, TextValue(value))),
        )
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Fault> {
        self.deserialize_map(visitor)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Fault> {
        self.single()?.deserialize_enum(name, variants, visitor)
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Fault> {
        visitor.visit_unit()
    }
}
