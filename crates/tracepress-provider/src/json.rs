#![allow(
    clippy::redundant_pub_crate,
    reason = "sibling parser modules share this bounded JSON interpretation"
)]

//! Bounded JSON interpretation shared by every provider parser.
//!
//! One document is materialised exactly once: nesting depth, member counts, and duplicate object
//! keys are enforced while the value is built, so an oversized shape is refused before the whole
//! tree exists. Retained scalars are bounded separately at extraction time, because the size of a
//! string the parser never keeps cannot threaten the observation.

use serde::de::{DeserializeSeed, Deserializer, Error as DeError, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};

use crate::domain::{ObservationError, ObservationInput, ObservationLimits};

/// Rejects bytes that can never be interpreted as bounded provider text.
pub(crate) fn validate_text(input: ObservationInput<'_>) -> Result<(), ObservationError> {
    if input.bytes.len() > input.limits.max_semantic_bytes {
        return Err(ObservationError::ResourceLimit);
    }
    if input.bytes.contains(&0) || std::str::from_utf8(input.bytes).is_err() {
        return Err(ObservationError::InvalidText);
    }
    Ok(())
}

/// Materialises one bounded JSON document exactly once.
///
/// Nesting deeper than `max_json_depth` or more members than `max_json_items` yield
/// [`ObservationError::ResourceLimit`]; invalid JSON, trailing bytes, and duplicate object keys
/// yield [`ObservationError::Malformed`].
pub(crate) fn parse_bounded(
    bytes: &[u8],
    limits: ObservationLimits,
) -> Result<Value, ObservationError> {
    let mut budget = Budget {
        items: 0,
        max_items: limits.max_json_items,
        max_depth: limits.max_json_depth,
        exhausted: false,
    };
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let parsed = BoundedValue {
        budget: &mut budget,
        depth: 0,
    }
    .deserialize(&mut deserializer)
    .and_then(|value| deserializer.end().map(|()| value));
    match parsed {
        Ok(value) => Ok(value),
        Err(_) if budget.exhausted => Err(ObservationError::ResourceLimit),
        Err(_) => Err(ObservationError::Malformed),
    }
}

/// Accounting shared by every level of one bounded document.
struct Budget {
    items: usize,
    max_items: usize,
    max_depth: usize,
    exhausted: bool,
}

impl Budget {
    fn add_item<E>(&mut self) -> Result<(), E>
    where
        E: DeError,
    {
        self.items = self.items.saturating_add(1);
        if self.items > self.max_items {
            self.exhausted = true;
            return Err(E::custom("bounded json item limit reached"));
        }
        Ok(())
    }

    fn enter<E>(&mut self, depth: usize) -> Result<usize, E>
    where
        E: DeError,
    {
        let next = depth.saturating_add(1);
        if next > self.max_depth {
            self.exhausted = true;
            return Err(E::custom("bounded json depth limit reached"));
        }
        Ok(next)
    }
}

/// Builds a [`Value`] while charging one shared [`Budget`].
struct BoundedValue<'a> {
    budget: &'a mut Budget,
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for BoundedValue<'_> {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for BoundedValue<'_> {
    type Value = Value;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("bounded JSON without duplicate object keys")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(Value::Null)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(Value::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(self)
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(Value::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(Value::Number(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Number::from_f64(value).map(Value::Number).ok_or_else(|| {
            E::custom("json number is not representable as a finite double-precision value")
        })
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(Value::String(value))
    }

    fn visit_seq<A>(self, mut access: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let depth = self.budget.enter(self.depth)?;
        let mut values = Vec::new();
        while let Some(value) = access.next_element_seed(BoundedValue {
            budget: &mut *self.budget,
            depth,
        })? {
            self.budget.add_item()?;
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let depth = self.budget.enter(self.depth)?;
        let mut map = Map::new();
        while let Some(key) = access.next_key::<String>()? {
            self.budget.add_item()?;
            let value = access.next_value_seed(BoundedValue {
                budget: &mut *self.budget,
                depth,
            })?;
            if map.insert(key, value).is_some() {
                return Err(A::Error::custom("duplicate json object key"));
            }
        }
        Ok(Value::Object(map))
    }
}

/// Returns the exact bytes of one member value of the JSON object serialised at `bytes`.
///
/// Only members of that object are considered, so a key of the same name nested inside any member
/// value is never returned. The scan trusts the well-formedness already established by
/// [`parse_bounded`] and only recovers a byte span; it never interprets the value.
pub(crate) fn member_span<'a>(bytes: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let mut remaining = skip_ascii_whitespace(bytes).strip_prefix(b"{")?;
    loop {
        remaining = skip_ascii_whitespace(remaining);
        let (name, after_key) = json_string_contents(remaining.strip_prefix(b"\"")?)?;
        let after_colon = skip_ascii_whitespace(after_key).strip_prefix(b":")?;
        let (value, after_value) = json_value_span(skip_ascii_whitespace(after_colon))?;
        if name == key {
            return Some(value);
        }
        remaining = skip_ascii_whitespace(after_value).strip_prefix(b",")?;
    }
}

const fn skip_ascii_whitespace(mut bytes: &[u8]) -> &[u8] {
    while let Some((&byte, rest)) = bytes.split_first() {
        if !byte.is_ascii_whitespace() {
            break;
        }
        bytes = rest;
    }
    bytes
}

/// Splits the contents of a JSON string, starting after its opening quote, from the bytes after it.
fn json_string_contents(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    let mut escaped = false;
    for (offset, &byte) in bytes.iter().enumerate() {
        if escaped {
            escaped = false;
        } else if byte == b'\\' {
            escaped = true;
        } else if byte == b'"' {
            return Some((bytes.get(..offset)?, bytes.get(offset.checked_add(1)?..)?));
        }
    }
    None
}

/// Splits one JSON value, starting at its first byte, from the bytes after it.
fn json_value_span(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    let length = match bytes.first()? {
        b'{' | b'[' => container_length(bytes)?,
        b'"' => {
            let (contents, _) = json_string_contents(bytes.get(1..)?)?;
            contents.len().checked_add(2)?
        }
        _ => bytes
            .iter()
            .position(|byte| matches!(byte, b',' | b'}' | b']') || byte.is_ascii_whitespace())
            .unwrap_or(bytes.len()),
    };
    if length == 0 {
        return None;
    }
    Some((bytes.get(..length)?, bytes.get(length..)?))
}

/// Returns the length of the balanced JSON object or array starting at `bytes`.
fn container_length(bytes: &[u8]) -> Option<usize> {
    let mut depth = 0_usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, &byte) in bytes.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => depth = depth.checked_add(1)?,
            b'}' | b']' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return offset.checked_add(1);
                }
            }
            _ => {}
        }
    }
    None
}

/// Bounded extraction of the allowlisted scalar strings an observation retains.
pub(crate) struct StringExtraction {
    max: usize,
    /// Whether a known field was present but could not be retained.
    pub(crate) partial: bool,
}

impl StringExtraction {
    pub(crate) const fn new(max: usize) -> Self {
        Self {
            max,
            partial: false,
        }
    }

    /// Retains one allowlisted string member, dropping a value this observation must not keep.
    ///
    /// A value of the wrong type, longer than the configured retention bound, or carrying an
    /// embedded NUL is dropped and marks the observation partial; the rest of the document is
    /// still interpreted.
    pub(crate) fn extract(&mut self, map: &Map<String, Value>, key: &str) -> Option<String> {
        match map.get(key) {
            None | Some(Value::Null) => None,
            Some(Value::String(value))
                if value.len() <= self.max && !value.as_bytes().contains(&0) =>
            {
                Some(value.clone())
            }
            Some(_) => {
                self.partial = true;
                None
            }
        }
    }

    /// Retains one allowlisted string member of a nested object.
    pub(crate) fn extract_nested(
        &mut self,
        map: &Map<String, Value>,
        field: NestedField,
    ) -> Option<String> {
        match map.get(field.parent) {
            None | Some(Value::Null) => None,
            Some(Value::Object(value)) => self.extract(value, field.key),
            Some(_) => {
                self.partial = true;
                None
            }
        }
    }
}

/// One allowlisted string member addressed through its parent object.
#[derive(Clone, Copy)]
pub(crate) struct NestedField {
    parent: &'static str,
    key: &'static str,
}

impl NestedField {
    pub(crate) const fn new(parent: &'static str, key: &'static str) -> Self {
        Self { parent, key }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> ObservationLimits {
        ObservationLimits::default()
    }

    #[test]
    fn duplicate_object_keys_are_malformed_at_every_depth() {
        assert_eq!(
            parse_bounded(br#"{"a":1,"a":2}"#, limits()),
            Err(ObservationError::Malformed)
        );
        assert_eq!(
            parse_bounded(br#"{"outer":{"b":1,"b":2}}"#, limits()),
            Err(ObservationError::Malformed)
        );
        assert!(parse_bounded(br#"{"a":1,"b":2}"#, limits()).is_ok());
    }

    #[test]
    fn member_span_returns_only_top_level_member_bytes() {
        let body = br#"{"nested":{"usage":{"input_tokens":1}},"note":"usage","usage": { "input_tokens": 7 } }"#;
        assert_eq!(
            member_span(body, b"usage"),
            Some(&br#"{ "input_tokens": 7 }"#[..])
        );
        assert_eq!(member_span(body, b"note"), Some(&br#""usage""#[..]));
        assert_eq!(member_span(body, b"absent"), None);
    }

    #[test]
    fn member_span_skips_every_scalar_and_container_member_shape() {
        let body = br#"{"a":[1,{"usage":2}],"b":-1.5e3,"c":null,"d":true,"e":"a\"b","usage":{}}"#;
        assert_eq!(member_span(body, b"usage"), Some(&b"{}"[..]));
    }

    #[test]
    fn depth_and_item_bounds_report_a_resource_limit() {
        let deep = ObservationLimits::new(crate::ObservationLimitValues {
            max_semantic_bytes: 1024,
            max_usage_bytes: 1024,
            max_sse_event_bytes: 1024,
            max_sse_events: 8,
            max_json_depth: 3,
            max_json_items: 4,
            max_string_bytes: 16,
        })
        .unwrap_or_default();
        assert_eq!(
            parse_bounded(br"[[[[1]]]]", deep),
            Err(ObservationError::ResourceLimit)
        );
        assert_eq!(
            parse_bounded(br"[1,2,3,4,5,6]", deep),
            Err(ObservationError::ResourceLimit)
        );
    }

    #[test]
    fn trailing_bytes_and_invalid_json_are_malformed() {
        assert_eq!(
            parse_bounded(br"{}{}", limits()),
            Err(ObservationError::Malformed)
        );
        assert_eq!(
            parse_bounded(br"{]", limits()),
            Err(ObservationError::Malformed)
        );
    }
}
