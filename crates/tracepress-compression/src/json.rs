//! JSON-only shadow candidates.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::{
    BlockMetadata, CompressionLimits, CompressorId, CompressorVersion, ShadowCompressor, Transform,
    TransformError, bounded_vec, validate_recovery_inputs,
};

/// J0 infrastructure control.
#[derive(Clone, Copy, Debug, Default)]
pub struct JsonNoop;

/// J1 structural-whitespace minification.
#[derive(Clone, Copy, Debug, Default)]
pub struct JsonMinify;

/// J2 homogeneous array-of-objects tabular projection.
#[derive(Clone, Copy, Debug, Default)]
pub struct JsonTabular;

/// J5 human-readable table projection for homogeneous JSON rows.
#[derive(Clone, Copy, Debug, Default)]
pub struct JsonReadableTable;

/// J6 compact, human-readable records with a fields header.
#[derive(Clone, Copy, Debug, Default)]
pub struct JsonCompactRecords;

/// J7 key-elision records with an explicit schema header.
#[derive(Clone, Copy, Debug, Default)]
pub struct JsonKeyElision;

/// J3 exact repeated-subtree factoring.
#[derive(Clone, Copy, Debug, Default)]
pub struct JsonRepeatedSubtree;

impl ShadowCompressor for JsonNoop {
    fn id(&self) -> CompressorId {
        CompressorId("json.noop")
    }

    fn version(&self) -> CompressorVersion {
        CompressorVersion(1)
    }

    fn supports(&self, metadata: BlockMetadata) -> bool {
        metadata.is_tool_result_json()
    }

    fn transform(
        &self,
        input: &[u8],
        limits: &CompressionLimits,
    ) -> Result<Transform, TransformError> {
        Ok(Transform::new(bounded_vec(input, limits)?, Vec::new()))
    }

    fn recover(
        &self,
        candidate: &[u8],
        _recovery: &[u8],
        limits: &CompressionLimits,
    ) -> Result<Box<[u8]>, TransformError> {
        Ok(bounded_vec(candidate, limits)?.into_boxed_slice())
    }
}

impl ShadowCompressor for JsonMinify {
    fn id(&self) -> CompressorId {
        CompressorId("json.minify")
    }

    fn version(&self) -> CompressorVersion {
        CompressorVersion(1)
    }

    fn supports(&self, metadata: BlockMetadata) -> bool {
        metadata.is_tool_result_json()
    }

    fn transform(
        &self,
        input: &[u8],
        limits: &CompressionLimits,
    ) -> Result<Transform, TransformError> {
        validate_json(input, limits)?;
        let mut candidate = Vec::with_capacity(input.len());
        let mut recovery = Vec::new();
        let mut in_string = false;
        let mut escaped = false;
        let mut index = 0_usize;
        while index < input.len() {
            let byte = *input.get(index).ok_or(TransformError::Internal)?;
            if !in_string && byte.is_ascii_whitespace() {
                let start = index;
                while input.get(index).is_some_and(u8::is_ascii_whitespace) {
                    index = index.saturating_add(1);
                }
                write_varint(
                    &mut recovery,
                    u64::try_from(candidate.len()).unwrap_or(u64::MAX),
                );
                let whitespace = input.get(start..index).ok_or(TransformError::Internal)?;
                write_bytes(&mut recovery, whitespace);
                continue;
            }
            candidate.push(byte);
            if in_string {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    in_string = false;
                }
            } else if byte == b'"' {
                in_string = true;
            }
            index = index.saturating_add(1);
        }
        bounds(&candidate, &recovery, limits)?;
        Ok(Transform::new(candidate, recovery))
    }

    fn recover(
        &self,
        candidate: &[u8],
        recovery: &[u8],
        limits: &CompressionLimits,
    ) -> Result<Box<[u8]>, TransformError> {
        validate_recovery_inputs(candidate, recovery, limits)?;
        let mut cursor = 0_usize;
        let mut output = Vec::with_capacity(candidate.len().saturating_add(recovery.len()));
        let mut source = 0_usize;
        while cursor < recovery.len() {
            let position = read_usize(recovery, &mut cursor)?;
            if position < source || position > candidate.len() {
                return Err(TransformError::InvalidInput);
            }
            output.extend_from_slice(
                candidate
                    .get(source..position)
                    .ok_or(TransformError::InvalidInput)?,
            );
            let whitespace = read_bytes(recovery, &mut cursor)?;
            output.extend_from_slice(whitespace);
            source = position;
            check_memory(output.len(), limits)?;
        }
        output.extend_from_slice(
            candidate
                .get(source..)
                .ok_or(TransformError::InvalidInput)?,
        );
        check_memory(output.len(), limits)?;
        Ok(output.into_boxed_slice())
    }
}

impl ShadowCompressor for JsonTabular {
    fn id(&self) -> CompressorId {
        CompressorId("json.tabular")
    }

    fn version(&self) -> CompressorVersion {
        CompressorVersion(1)
    }

    fn supports(&self, metadata: BlockMetadata) -> bool {
        metadata.is_tool_result_json()
    }

    fn transform(
        &self,
        input: &[u8],
        limits: &CompressionLimits,
    ) -> Result<Transform, TransformError> {
        validate_json(input, limits)?;
        let value: Value =
            serde_json::from_slice(input).map_err(|_| TransformError::InvalidInput)?;
        let Value::Array(rows) = value else {
            return Err(TransformError::NotApplicable);
        };
        if rows.len() < 2 {
            return Err(TransformError::NotApplicable);
        }
        let first = rows
            .first()
            .and_then(Value::as_object)
            .ok_or(TransformError::NotApplicable)?;
        if first.is_empty() {
            return Err(TransformError::NotApplicable);
        }
        let columns = first.keys().cloned().collect::<Vec<_>>();
        let expected = columns.iter().cloned().collect::<BTreeSet<_>>();
        let mut projected = Vec::with_capacity(rows.len());
        for row in &rows {
            let object = row.as_object().ok_or(TransformError::NotApplicable)?;
            if object.keys().cloned().collect::<BTreeSet<_>>() != expected {
                return Err(TransformError::NotApplicable);
            }
            projected.push(
                columns
                    .iter()
                    .map(|column| object.get(column).cloned().ok_or(TransformError::Internal))
                    .collect::<Result<Vec<_>, _>>()?,
            );
        }
        let candidate = serde_json::to_vec(&("TPJ2", columns, projected))
            .map_err(|_| TransformError::Internal)?;
        let recovery = bounded_vec(input, limits)?;
        bounds(&candidate, &recovery, limits)?;
        Ok(Transform::new(candidate, recovery))
    }

    fn recover(
        &self,
        candidate: &[u8],
        recovery: &[u8],
        limits: &CompressionLimits,
    ) -> Result<Box<[u8]>, TransformError> {
        validate_recovery_inputs(candidate, recovery, limits)?;
        let _: Value =
            serde_json::from_slice(candidate).map_err(|_| TransformError::InvalidInput)?;
        Ok(bounded_vec(recovery, limits)?.into_boxed_slice())
    }
}

impl ShadowCompressor for JsonReadableTable {
    fn id(&self) -> CompressorId {
        CompressorId("json.readable_table")
    }

    fn version(&self) -> CompressorVersion {
        CompressorVersion(1)
    }

    fn supports(&self, metadata: BlockMetadata) -> bool {
        metadata.is_tool_result_json()
    }

    fn transform(
        &self,
        input: &[u8],
        limits: &CompressionLimits,
    ) -> Result<Transform, TransformError> {
        readable_table_transform(input, limits, ReadableTableStyle::HumanTable)
    }

    fn recover(
        &self,
        candidate: &[u8],
        recovery: &[u8],
        limits: &CompressionLimits,
    ) -> Result<Box<[u8]>, TransformError> {
        readable_table_recover(candidate, recovery, limits, ReadableTableStyle::HumanTable)
    }
}

impl ShadowCompressor for JsonCompactRecords {
    fn id(&self) -> CompressorId {
        CompressorId("json.compact_records")
    }

    fn version(&self) -> CompressorVersion {
        CompressorVersion(1)
    }

    fn supports(&self, metadata: BlockMetadata) -> bool {
        metadata.is_tool_result_json()
    }

    fn transform(
        &self,
        input: &[u8],
        limits: &CompressionLimits,
    ) -> Result<Transform, TransformError> {
        readable_table_transform(input, limits, ReadableTableStyle::CompactRecords)
    }

    fn recover(
        &self,
        candidate: &[u8],
        recovery: &[u8],
        limits: &CompressionLimits,
    ) -> Result<Box<[u8]>, TransformError> {
        readable_table_recover(
            candidate,
            recovery,
            limits,
            ReadableTableStyle::CompactRecords,
        )
    }
}

impl ShadowCompressor for JsonKeyElision {
    fn id(&self) -> CompressorId {
        CompressorId("json.key_elision")
    }

    fn version(&self) -> CompressorVersion {
        CompressorVersion(1)
    }

    fn supports(&self, metadata: BlockMetadata) -> bool {
        metadata.is_tool_result_json()
    }

    fn transform(
        &self,
        input: &[u8],
        limits: &CompressionLimits,
    ) -> Result<Transform, TransformError> {
        readable_table_transform(input, limits, ReadableTableStyle::KeyElision)
    }

    fn recover(
        &self,
        candidate: &[u8],
        recovery: &[u8],
        limits: &CompressionLimits,
    ) -> Result<Box<[u8]>, TransformError> {
        readable_table_recover(candidate, recovery, limits, ReadableTableStyle::KeyElision)
    }
}

#[derive(Clone, Copy)]
enum ReadableTableStyle {
    HumanTable,
    CompactRecords,
    KeyElision,
}

fn readable_table_transform(
    input: &[u8],
    limits: &CompressionLimits,
    style: ReadableTableStyle,
) -> Result<Transform, TransformError> {
    validate_json(input, limits)?;
    let value: Value = serde_json::from_slice(input).map_err(|_| TransformError::InvalidInput)?;
    let Value::Array(rows) = value else {
        return Err(TransformError::NotApplicable);
    };
    if rows.len() < 2 || rows.len() > 100_000 {
        return Err(if rows.len() > 100_000 {
            TransformError::ResourceLimit
        } else {
            TransformError::NotApplicable
        });
    }
    let first = rows
        .first()
        .and_then(Value::as_object)
        .ok_or(TransformError::NotApplicable)?;
    if first.is_empty() || first.len() > 64 {
        return Err(TransformError::NotApplicable);
    }
    let columns = first.keys().cloned().collect::<Vec<_>>();
    let expected = columns.iter().cloned().collect::<BTreeSet<_>>();
    let mut values = Vec::with_capacity(rows.len());
    let mut work = 0_u64;
    for row in &rows {
        let object = row.as_object().ok_or(TransformError::NotApplicable)?;
        if object.keys().cloned().collect::<BTreeSet<_>>() != expected {
            return Err(TransformError::NotApplicable);
        }
        let mut cells = Vec::with_capacity(columns.len());
        for column in &columns {
            let cell = object
                .get(column)
                .cloned()
                .ok_or(TransformError::Internal)?;
            let depth = json_value_depth(&cell, 0, limits, &mut work)?;
            if depth > 8 {
                return Err(TransformError::NotApplicable);
            }
            cells.push(cell);
        }
        values.push(cells);
        if work > limits.max_shadow_work_units {
            return Err(TransformError::ResourceLimit);
        }
    }
    let candidate = render_readable_table(&columns, &values, style, limits)?;
    let recovery = bounded_vec(input, limits)?;
    bounds(&candidate, &recovery, limits)?;
    Ok(Transform::new(candidate, recovery))
}

fn readable_table_recover(
    candidate: &[u8],
    recovery: &[u8],
    limits: &CompressionLimits,
    style: ReadableTableStyle,
) -> Result<Box<[u8]>, TransformError> {
    validate_recovery_inputs(candidate, recovery, limits)?;
    let prefix: &[u8] = match style {
        ReadableTableStyle::HumanTable => b"Tracepress table v1\n",
        ReadableTableStyle::CompactRecords => b"Tracepress records v1\n",
        ReadableTableStyle::KeyElision => b"Tracepress key-elision v1\n",
    };
    if !candidate.starts_with(prefix) || std::str::from_utf8(candidate).is_err() {
        return Err(TransformError::InvalidInput);
    }
    Ok(bounded_vec(recovery, limits)?.into_boxed_slice())
}

fn render_readable_table(
    columns: &[String],
    rows: &[Vec<Value>],
    style: ReadableTableStyle,
    limits: &CompressionLimits,
) -> Result<Vec<u8>, TransformError> {
    let mut output = Vec::new();
    let prefix = match style {
        ReadableTableStyle::HumanTable => b"Tracepress table v1\n".as_slice(),
        ReadableTableStyle::CompactRecords => b"Tracepress records v1\n".as_slice(),
        ReadableTableStyle::KeyElision => b"Tracepress key-elision v1\n".as_slice(),
    };
    output.extend_from_slice(prefix);
    match style {
        ReadableTableStyle::HumanTable => {
            output.extend_from_slice(b"fields\t");
            write_json(
                &mut output,
                &Value::Array(columns.iter().cloned().map(Value::String).collect()),
            )?;
            output.push(b'\n');
            for row in rows {
                output.extend_from_slice(b"row\t");
                write_json(&mut output, &Value::Array(row.clone()))?;
                output.push(b'\n');
            }
        }
        ReadableTableStyle::CompactRecords => {
            output.extend_from_slice(b"fields: ");
            write_json(
                &mut output,
                &Value::Array(columns.iter().cloned().map(Value::String).collect()),
            )?;
            output.push(b'\n');
            for row in rows {
                write_json(&mut output, &Value::Array(row.clone()))?;
                output.push(b'\n');
            }
        }
        ReadableTableStyle::KeyElision => {
            output.extend_from_slice(b"fields(");
            for (index, column) in columns.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                write_json(&mut output, &Value::String(column.clone()))?;
            }
            output.extend_from_slice(b"): \n");
            for row in rows {
                output.push(b'(');
                for (index, cell) in row.iter().enumerate() {
                    if index > 0 {
                        output.push(b',');
                    }
                    write_json(&mut output, cell)?;
                }
                output.extend_from_slice(b")\n");
            }
        }
    }
    if u64::try_from(output.len()).unwrap_or(u64::MAX) > limits.max_candidate_output_bytes {
        return Err(TransformError::ResourceLimit);
    }
    Ok(output)
}

fn write_json(output: &mut Vec<u8>, value: &Value) -> Result<(), TransformError> {
    let bytes = serde_json::to_vec(value).map_err(|_| TransformError::Internal)?;
    output.extend_from_slice(&bytes);
    Ok(())
}

fn json_value_depth(
    value: &Value,
    depth: u32,
    limits: &CompressionLimits,
    work: &mut u64,
) -> Result<u32, TransformError> {
    *work = work.saturating_add(1);
    if *work > limits.max_shadow_work_units {
        return Err(TransformError::ResourceLimit);
    }
    match value {
        Value::Array(values) => values.iter().try_fold(depth, |maximum, value| {
            json_value_depth(value, depth.saturating_add(1), limits, work)
                .map(|child| maximum.max(child))
        }),
        Value::Object(values) => values.values().try_fold(depth, |maximum, value| {
            json_value_depth(value, depth.saturating_add(1), limits, work)
                .map(|child| maximum.max(child))
        }),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => Ok(depth),
    }
}

impl ShadowCompressor for JsonRepeatedSubtree {
    fn id(&self) -> CompressorId {
        CompressorId("json.repeated_subtree")
    }

    fn version(&self) -> CompressorVersion {
        CompressorVersion(1)
    }

    fn supports(&self, metadata: BlockMetadata) -> bool {
        metadata.is_tool_result_json()
    }

    fn transform(
        &self,
        input: &[u8],
        limits: &CompressionLimits,
    ) -> Result<Transform, TransformError> {
        validate_json(input, limits)?;
        let value: Value =
            serde_json::from_slice(input).map_err(|_| TransformError::InvalidInput)?;
        let mut frequencies = BTreeMap::<Vec<u8>, u64>::new();
        collect_subtrees(&value, &mut frequencies, limits, &mut 0, &mut 0)?;
        let definitions = frequencies
            .into_iter()
            .filter(|(bytes, count)| *count > 1 && bytes.len() >= 8)
            .map(|(bytes, _count)| serde_json::from_slice::<Value>(&bytes))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| TransformError::Internal)?;
        if definitions.is_empty() {
            return Err(TransformError::NotApplicable);
        }
        let indices = definitions
            .iter()
            .enumerate()
            .map(|(index, definition)| {
                serde_json::to_vec(definition)
                    .map(|bytes| (bytes, index))
                    .map_err(|_| TransformError::Internal)
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let transformed = factor_value(value, &indices, true, limits, &mut 0)?;
        let candidate = serde_json::to_vec(&("TPJ3", definitions, transformed))
            .map_err(|_| TransformError::Internal)?;
        let recovery = bounded_vec(input, limits)?;
        bounds(&candidate, &recovery, limits)?;
        Ok(Transform::new(candidate, recovery))
    }

    fn recover(
        &self,
        candidate: &[u8],
        recovery: &[u8],
        limits: &CompressionLimits,
    ) -> Result<Box<[u8]>, TransformError> {
        validate_recovery_inputs(candidate, recovery, limits)?;
        let _: Value =
            serde_json::from_slice(candidate).map_err(|_| TransformError::InvalidInput)?;
        Ok(bounded_vec(recovery, limits)?.into_boxed_slice())
    }
}

#[allow(
    clippy::redundant_pub_crate,
    reason = "the strict validator is shared only by sibling modules inside this private crate module"
)]
pub(crate) fn validate_json(
    input: &[u8],
    limits: &CompressionLimits,
) -> Result<(), TransformError> {
    if u64::try_from(input.len()).unwrap_or(u64::MAX) > limits.max_shadow_work_units {
        return Err(TransformError::ResourceLimit);
    }
    let _: Value = serde_json::from_slice(input).map_err(|_| TransformError::InvalidInput)?;
    let mut parser = DuplicateParser {
        input,
        offset: 0,
        work: 0,
        max_work: limits.max_shadow_work_units,
    };
    parser.parse_value(0)?;
    parser.whitespace()?;
    if parser.offset != input.len() {
        return Err(TransformError::InvalidInput);
    }
    Ok(())
}

struct DuplicateParser<'input> {
    input: &'input [u8],
    offset: usize,
    work: u64,
    max_work: u64,
}

impl DuplicateParser<'_> {
    fn parse_value(&mut self, depth: u16) -> Result<(), TransformError> {
        if depth > 128 {
            return Err(TransformError::ResourceLimit);
        }
        self.whitespace()?;
        match self.peek()? {
            b'{' => self.object(depth.saturating_add(1)),
            b'[' => self.array(depth.saturating_add(1)),
            b'"' => self.string().map(|_| ()),
            _ => self.scalar(),
        }
    }

    fn object(&mut self, depth: u16) -> Result<(), TransformError> {
        self.take(b'{')?;
        self.whitespace()?;
        let mut keys = BTreeSet::new();
        if self.peek()? == b'}' {
            return self.take(b'}');
        }
        loop {
            self.whitespace()?;
            let key = self.string()?;
            let decoded: String =
                serde_json::from_slice(key).map_err(|_| TransformError::InvalidInput)?;
            if !keys.insert(decoded) {
                return Err(TransformError::NotApplicable);
            }
            self.whitespace()?;
            self.take(b':')?;
            self.parse_value(depth)?;
            self.whitespace()?;
            match self.peek()? {
                b',' => self.take(b',')?,
                b'}' => return self.take(b'}'),
                _ => return Err(TransformError::InvalidInput),
            }
        }
    }

    fn array(&mut self, depth: u16) -> Result<(), TransformError> {
        self.take(b'[')?;
        self.whitespace()?;
        if self.peek()? == b']' {
            return self.take(b']');
        }
        loop {
            self.parse_value(depth)?;
            self.whitespace()?;
            match self.peek()? {
                b',' => self.take(b',')?,
                b']' => return self.take(b']'),
                _ => return Err(TransformError::InvalidInput),
            }
        }
    }

    fn string(&mut self) -> Result<&[u8], TransformError> {
        let start = self.offset;
        self.take(b'"')?;
        let mut escaped = false;
        loop {
            let byte = self.next()?;
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                return self
                    .input
                    .get(start..self.offset)
                    .ok_or(TransformError::Internal);
            }
        }
    }

    fn scalar(&mut self) -> Result<(), TransformError> {
        let start = self.offset;
        while self
            .input
            .get(self.offset)
            .is_some_and(|byte| !byte.is_ascii_whitespace() && !matches!(byte, b',' | b'}' | b']'))
        {
            let _ = self.next()?;
        }
        if self.offset == start {
            Err(TransformError::InvalidInput)
        } else {
            Ok(())
        }
    }

    fn whitespace(&mut self) -> Result<(), TransformError> {
        while self
            .input
            .get(self.offset)
            .is_some_and(u8::is_ascii_whitespace)
        {
            let _ = self.next()?;
        }
        Ok(())
    }

    fn peek(&self) -> Result<u8, TransformError> {
        self.input
            .get(self.offset)
            .copied()
            .ok_or(TransformError::InvalidInput)
    }

    fn take(&mut self, expected: u8) -> Result<(), TransformError> {
        if self.next()? == expected {
            Ok(())
        } else {
            Err(TransformError::InvalidInput)
        }
    }

    fn next(&mut self) -> Result<u8, TransformError> {
        self.work = self.work.saturating_add(1);
        if self.work > self.max_work {
            return Err(TransformError::ResourceLimit);
        }
        let byte = self
            .input
            .get(self.offset)
            .copied()
            .ok_or(TransformError::InvalidInput)?;
        self.offset = self.offset.saturating_add(1);
        Ok(byte)
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "recursive traversal carries explicit shared bounds and accounting"
)]
fn collect_subtrees(
    value: &Value,
    frequencies: &mut BTreeMap<Vec<u8>, u64>,
    limits: &CompressionLimits,
    work: &mut u64,
    retained_bytes: &mut u64,
) -> Result<(), TransformError> {
    *work = work.saturating_add(1);
    if *work > limits.max_shadow_work_units {
        return Err(TransformError::ResourceLimit);
    }
    match value {
        Value::Array(values) => {
            for child in values {
                collect_subtrees(child, frequencies, limits, work, retained_bytes)?;
            }
        }
        Value::Object(values) => {
            for child in values.values() {
                collect_subtrees(child, frequencies, limits, work, retained_bytes)?;
            }
        }
        _ => return Ok(()),
    }
    let encoded = serde_json::to_vec(value).map_err(|_| TransformError::Internal)?;
    *work = work.saturating_add(u64::try_from(encoded.len()).unwrap_or(u64::MAX));
    if *work > limits.max_shadow_work_units {
        return Err(TransformError::ResourceLimit);
    }
    if !frequencies.contains_key(&encoded) {
        *retained_bytes = retained_bytes
            .checked_add(u64::try_from(encoded.len()).unwrap_or(u64::MAX))
            .ok_or(TransformError::ResourceLimit)?;
        if *retained_bytes > limits.max_shadow_memory_bytes {
            return Err(TransformError::ResourceLimit);
        }
    }
    let count = frequencies.entry(encoded).or_default();
    *count = count.saturating_add(1);
    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "recursive traversal carries explicit shared bounds and accounting"
)]
fn factor_value(
    value: Value,
    indices: &BTreeMap<Vec<u8>, usize>,
    root: bool,
    limits: &CompressionLimits,
    work: &mut u64,
) -> Result<Value, TransformError> {
    *work = work.saturating_add(1);
    if *work > limits.max_shadow_work_units {
        return Err(TransformError::ResourceLimit);
    }
    if !root {
        let encoded = serde_json::to_vec(&value).map_err(|_| TransformError::Internal)?;
        if let Some(index) = indices.get(&encoded) {
            return Ok(serde_json::json!(["$tp_ref", index]));
        }
    }
    match value {
        Value::Array(values) => values
            .into_iter()
            .map(|child| factor_value(child, indices, false, limits, work))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(values) => values
            .into_iter()
            .map(|(key, child)| {
                factor_value(child, indices, false, limits, work).map(|child| (key, child))
            })
            .collect::<Result<serde_json::Map<_, _>, _>>()
            .map(Value::Object),
        scalar => Ok(scalar),
    }
}

fn bounds(
    candidate: &[u8],
    recovery: &[u8],
    limits: &CompressionLimits,
) -> Result<(), TransformError> {
    let candidate = u64::try_from(candidate.len()).unwrap_or(u64::MAX);
    let recovery = u64::try_from(recovery.len()).unwrap_or(u64::MAX);
    if candidate > limits.max_candidate_output_bytes
        || candidate.saturating_add(recovery) > limits.max_shadow_memory_bytes
    {
        Err(TransformError::ResourceLimit)
    } else {
        Ok(())
    }
}

fn check_memory(length: usize, limits: &CompressionLimits) -> Result<(), TransformError> {
    if u64::try_from(length).unwrap_or(u64::MAX) > limits.max_shadow_memory_bytes {
        Err(TransformError::ResourceLimit)
    } else {
        Ok(())
    }
}

fn write_bytes(output: &mut Vec<u8>, bytes: &[u8]) {
    write_varint(output, u64::try_from(bytes.len()).unwrap_or(u64::MAX));
    output.extend_from_slice(bytes);
}

fn write_varint(output: &mut Vec<u8>, mut value: u64) {
    loop {
        let low = u8::try_from(value & 0x7f).unwrap_or(0);
        value >>= 7;
        if value == 0 {
            output.push(low);
            return;
        }
        output.push(low | 0x80);
    }
}

fn read_usize(input: &[u8], cursor: &mut usize) -> Result<usize, TransformError> {
    usize::try_from(read_varint(input, cursor)?).map_err(|_| TransformError::ResourceLimit)
}

fn read_bytes<'input>(
    input: &'input [u8],
    cursor: &mut usize,
) -> Result<&'input [u8], TransformError> {
    let length = read_usize(input, cursor)?;
    let end = cursor
        .checked_add(length)
        .ok_or(TransformError::ResourceLimit)?;
    let bytes = input
        .get(*cursor..end)
        .ok_or(TransformError::InvalidInput)?;
    *cursor = end;
    Ok(bytes)
}

fn read_varint(input: &[u8], cursor: &mut usize) -> Result<u64, TransformError> {
    let mut value = 0_u64;
    for shift in (0..=63).step_by(7) {
        let byte = input
            .get(*cursor)
            .copied()
            .ok_or(TransformError::InvalidInput)?;
        *cursor = cursor.saturating_add(1);
        if shift == 63 && byte & 0x7f > 1 {
            return Err(TransformError::InvalidInput);
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(TransformError::InvalidInput)
}

#[cfg(test)]
mod tests {
    use crate::{
        BlockKind, BlockMetadata, BlockOrigin, CandidateStatus, CompressionLimits, DetectedKind,
        evaluate,
    };

    use super::*;

    fn json_metadata() -> BlockMetadata {
        BlockMetadata {
            origin: BlockOrigin::ToolGenerated,
            kind: BlockKind::ToolResult,
            detected_kind: DetectedKind::Json,
            input_estimated_tokens: Some(1_000),
            request_offset: 200,
            request_analysis_bytes: 2_000,
            persistence: Some(3),
            exact_repetition: Some(true),
        }
    }

    #[test]
    fn minify_is_lossless_deterministic_and_rejects_duplicate_keys() {
        let input = br#"{
  "message": "space inside string",
  "items": [1, 2, 3]
}"#;
        let candidate = evaluate(
            &JsonMinify,
            json_metadata(),
            input,
            &CompressionLimits::default(),
        );
        assert_eq!(candidate.metrics().status, CandidateStatus::Applicable);
        assert!(candidate.metrics().recovery_verified);
        assert!(candidate.metrics().deterministic);
        let duplicate = evaluate(
            &JsonMinify,
            json_metadata(),
            br#"{"key":1,"key":2}"#,
            &CompressionLimits::default(),
        );
        assert_eq!(duplicate.metrics().status, CandidateStatus::NotApplicable);
    }

    #[test]
    fn structural_candidates_are_shape_limited_and_recoverable() {
        let tabular = br#"[{"very_long_column_name":"a","status":"ok"},{"very_long_column_name":"b","status":"ok"},{"very_long_column_name":"c","status":"ok"}]"#;
        let result = evaluate(
            &JsonTabular,
            json_metadata(),
            tabular,
            &CompressionLimits::default(),
        );
        assert!(matches!(
            result.metrics().status,
            CandidateStatus::Applicable | CandidateStatus::NoImprovement
        ));
        assert!(result.metrics().recovery_verified);

        let repeated = br#"{"a":{"long_repeated_value":[1,2,3,4]},"b":{"long_repeated_value":[1,2,3,4]},"c":{"long_repeated_value":[1,2,3,4]}}"#;
        let result = evaluate(
            &JsonRepeatedSubtree,
            json_metadata(),
            repeated,
            &CompressionLimits::default(),
        );
        assert!(matches!(
            result.metrics().status,
            CandidateStatus::Applicable | CandidateStatus::NoImprovement
        ));
        assert!(result.metrics().recovery_verified);
    }

    #[test]
    fn malformed_deep_and_huge_inputs_fail_closed() {
        let limits = CompressionLimits {
            max_candidate_input_bytes: 32,
            ..CompressionLimits::default()
        };
        let huge = evaluate(&JsonMinify, json_metadata(), &[b' '; 33], &limits);
        assert_eq!(huge.metrics().status, CandidateStatus::ResourceLimit);
        let malformed = evaluate(
            &JsonMinify,
            json_metadata(),
            b"{",
            &CompressionLimits::default(),
        );
        assert_eq!(malformed.metrics().status, CandidateStatus::InvalidInput);
    }

    #[test]
    fn repeated_subtree_retention_is_charged_before_insertion() {
        let value: Value = serde_json::from_slice(
            br#"{"outer":{"first":{"value":"abcdefgh"},"second":{"value":"ijklmnop"}}}"#,
        )
        .unwrap_or(Value::Null);
        let limits = CompressionLimits {
            max_shadow_memory_bytes: 24,
            ..CompressionLimits::default()
        };
        let result = collect_subtrees(&value, &mut BTreeMap::new(), &limits, &mut 0, &mut 0);
        assert_eq!(result, Err(TransformError::ResourceLimit));
    }

    #[test]
    fn minify_recovery_rejects_overflowing_varints() {
        let mut recovery = vec![0x80; 9];
        recovery.push(0x02);
        let result = JsonMinify.recover(&[], &recovery, &CompressionLimits::default());
        assert_eq!(result, Err(TransformError::InvalidInput));
    }

    #[test]
    fn readable_json_candidates_are_human_structured_and_recoverable() {
        let input = br#"[
          {"file":"src/very-long-module-name.rs","line":10,"severity":"error"},
          {"file":"src/another-very-long-module-name.rs","line":42,"severity":"warning"},
          {"file":"src/third-very-long-module-name.rs","line":84,"severity":"error"}
        ]"#;
        for (compressor, prefix) in [
            (
                &JsonReadableTable as &dyn ShadowCompressor,
                b"Tracepress table v1\n".as_slice(),
            ),
            (&JsonCompactRecords, b"Tracepress records v1\n".as_slice()),
            (&JsonKeyElision, b"Tracepress key-elision v1\n".as_slice()),
        ] {
            let result = evaluate(
                compressor,
                json_metadata(),
                input,
                &CompressionLimits::default(),
            );
            assert!(result.metrics().recovery_verified);
            assert!(result.metrics().deterministic);
            assert!(
                result
                    .payload()
                    .is_some_and(|bytes| bytes.starts_with(prefix))
            );
        }
    }

    #[test]
    fn readable_json_candidates_reject_heterogeneous_rows() {
        let input = br#"[{"a":1,"b":2},{"a":3}]"#;
        let result = evaluate(
            &JsonKeyElision,
            json_metadata(),
            input,
            &CompressionLimits::default(),
        );
        assert_eq!(result.metrics().status, CandidateStatus::NotApplicable);
    }
}
