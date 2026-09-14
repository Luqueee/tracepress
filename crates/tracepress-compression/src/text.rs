//! Plain-text `ToolResult` shadow candidates.

use crate::{
    BlockMetadata, CompressionLimits, CompressorId, CompressorVersion, ShadowCompressor, Transform,
    TransformError, bounded_vec, validate_recovery_inputs,
};
use serde_json::Value;

/// P0 infrastructure control.
#[derive(Clone, Copy, Debug, Default)]
pub struct TextNoop;

/// P1 consecutive identical-line folding.
#[derive(Clone, Copy, Debug, Default)]
pub struct TextRepeatedLine;

/// P2 bounded adjacent multi-line run folding.
#[derive(Clone, Copy, Debug, Default)]
pub struct TextRepeatedRun;

/// P3 explicit, human-readable duplicate-line folding.
#[derive(Clone, Copy, Debug, Default)]
pub struct TextReadableLineFold;

/// P4 explicit, human-readable repeated-block folding.
#[derive(Clone, Copy, Debug, Default)]
pub struct TextReadableBlockFold;

/// P5 conservative factoring for repeated structured log prefixes.
#[derive(Clone, Copy, Debug, Default)]
pub struct TextLogPrefixFold;

impl ShadowCompressor for TextNoop {
    fn id(&self) -> CompressorId {
        CompressorId("text.noop")
    }

    fn version(&self) -> CompressorVersion {
        CompressorVersion(1)
    }

    fn supports(&self, metadata: BlockMetadata) -> bool {
        metadata.is_tool_result_plain_text()
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

impl ShadowCompressor for TextRepeatedLine {
    fn id(&self) -> CompressorId {
        CompressorId("text.repeated_line")
    }

    fn version(&self) -> CompressorVersion {
        CompressorVersion(1)
    }

    fn supports(&self, metadata: BlockMetadata) -> bool {
        metadata.is_tool_result_plain_text()
    }

    fn transform(
        &self,
        input: &[u8],
        limits: &CompressionLimits,
    ) -> Result<Transform, TransformError> {
        validate_text(input, limits)?;
        let lines = lines(input);
        let mut records = Vec::new();
        let mut index = 0_usize;
        while index < lines.len() {
            let line = *lines.get(index).ok_or(TransformError::Internal)?;
            let mut count = 1_usize;
            while lines.get(index.saturating_add(count)).copied() == Some(line) {
                count = count.saturating_add(1);
            }
            records.push((count, vec![line]));
            index = index.saturating_add(count);
        }
        encode_records(*b"TPL1", &records, limits)
    }

    fn recover(
        &self,
        candidate: &[u8],
        _recovery: &[u8],
        limits: &CompressionLimits,
    ) -> Result<Box<[u8]>, TransformError> {
        decode_records(candidate, *b"TPL1", limits)
    }
}

impl ShadowCompressor for TextRepeatedRun {
    fn id(&self) -> CompressorId {
        CompressorId("text.repeated_run")
    }

    fn version(&self) -> CompressorVersion {
        CompressorVersion(1)
    }

    fn supports(&self, metadata: BlockMetadata) -> bool {
        metadata.is_tool_result_plain_text()
    }

    fn transform(
        &self,
        input: &[u8],
        limits: &CompressionLimits,
    ) -> Result<Transform, TransformError> {
        validate_text(input, limits)?;
        let lines = lines(input);
        let mut records = Vec::new();
        let mut index = 0_usize;
        let mut work = 0_u64;
        while index < lines.len() {
            let remaining = lines.len().saturating_sub(index);
            let mut best = None::<(usize, usize, usize)>;
            for pattern_len in 2..=remaining.saturating_div(2).min(8) {
                work = work.saturating_add(1);
                if work > limits.max_shadow_work_units {
                    return Err(TransformError::ResourceLimit);
                }
                let pattern = lines
                    .get(index..index.saturating_add(pattern_len))
                    .ok_or(TransformError::Internal)?;
                let mut repetitions = 1_usize;
                while lines.get(
                    index.saturating_add(repetitions.saturating_mul(pattern_len))
                        ..index.saturating_add(
                            repetitions.saturating_add(1).saturating_mul(pattern_len),
                        ),
                ) == Some(pattern)
                {
                    repetitions = repetitions.saturating_add(1);
                    work = work.saturating_add(u64::try_from(pattern_len).unwrap_or(u64::MAX));
                    if work > limits.max_shadow_work_units {
                        return Err(TransformError::ResourceLimit);
                    }
                }
                let covered = pattern_len.saturating_mul(repetitions);
                if repetitions > 1 && best.is_none_or(|(_, _, best_covered)| covered > best_covered)
                {
                    best = Some((pattern_len, repetitions, covered));
                }
            }
            if let Some((pattern_len, repetitions, covered)) = best {
                let pattern = lines
                    .get(index..index.saturating_add(pattern_len))
                    .ok_or(TransformError::Internal)?
                    .to_vec();
                records.push((repetitions, pattern));
                index = index.saturating_add(covered);
            } else {
                records.push((1, vec![*lines.get(index).ok_or(TransformError::Internal)?]));
                index = index.saturating_add(1);
            }
        }
        encode_records(*b"TPR2", &records, limits)
    }

    fn recover(
        &self,
        candidate: &[u8],
        _recovery: &[u8],
        limits: &CompressionLimits,
    ) -> Result<Box<[u8]>, TransformError> {
        decode_records(candidate, *b"TPR2", limits)
    }
}

impl ShadowCompressor for TextReadableLineFold {
    fn id(&self) -> CompressorId {
        CompressorId("text.readable_line_fold")
    }

    fn version(&self) -> CompressorVersion {
        CompressorVersion(1)
    }

    fn supports(&self, metadata: BlockMetadata) -> bool {
        metadata.is_tool_result_plain_text()
    }

    fn transform(
        &self,
        input: &[u8],
        limits: &CompressionLimits,
    ) -> Result<Transform, TransformError> {
        let lines = validated_lines(input, limits)?;
        if !lines.iter().any(|(_, count)| *count >= 2) {
            return Err(TransformError::NotApplicable);
        }
        let mut candidate = b"Tracepress line-fold v1\n".to_vec();
        for (line, count) in lines {
            if count >= 2 {
                candidate.extend_from_slice(format!("repeat={count} line=").as_bytes());
            } else {
                candidate.extend_from_slice(b"line=");
            }
            let line = std::str::from_utf8(line).map_err(|_| TransformError::InvalidInput)?;
            write_json_string(&mut candidate, line)?;
            candidate.push(b'\n');
        }
        readable_text_bounds(&candidate, input, limits)?;
        Ok(Transform::new(candidate, bounded_vec(input, limits)?))
    }

    fn recover(
        &self,
        candidate: &[u8],
        recovery: &[u8],
        limits: &CompressionLimits,
    ) -> Result<Box<[u8]>, TransformError> {
        recover_readable_text(candidate, recovery, b"Tracepress line-fold v1\n", limits)
    }
}

impl ShadowCompressor for TextReadableBlockFold {
    fn id(&self) -> CompressorId {
        CompressorId("text.readable_block_fold")
    }

    fn version(&self) -> CompressorVersion {
        CompressorVersion(1)
    }

    fn supports(&self, metadata: BlockMetadata) -> bool {
        metadata.is_tool_result_plain_text()
    }

    fn transform(
        &self,
        input: &[u8],
        limits: &CompressionLimits,
    ) -> Result<Transform, TransformError> {
        let lines = validated_line_values(input, limits)?;
        let records = repeated_block_records(&lines, limits)?;
        if !records.iter().any(|record| record.repetitions >= 2) {
            return Err(TransformError::NotApplicable);
        }
        let mut candidate = b"Tracepress block-fold v1\n".to_vec();
        for record in records {
            if record.repetitions >= 2 {
                candidate
                    .extend_from_slice(format!("repeat={} lines=", record.repetitions).as_bytes());
            } else {
                candidate.extend_from_slice(b"lines=");
            }
            let values = record
                .lines
                .into_iter()
                .map(Value::String)
                .collect::<Vec<_>>();
            let encoded = serde_json::to_vec(&values).map_err(|_| TransformError::Internal)?;
            candidate.extend_from_slice(&encoded);
            candidate.push(b'\n');
        }
        readable_text_bounds(&candidate, input, limits)?;
        Ok(Transform::new(candidate, bounded_vec(input, limits)?))
    }

    fn recover(
        &self,
        candidate: &[u8],
        recovery: &[u8],
        limits: &CompressionLimits,
    ) -> Result<Box<[u8]>, TransformError> {
        recover_readable_text(candidate, recovery, b"Tracepress block-fold v1\n", limits)
    }
}

impl ShadowCompressor for TextLogPrefixFold {
    fn id(&self) -> CompressorId {
        CompressorId("text.log_prefix_fold")
    }

    fn version(&self) -> CompressorVersion {
        CompressorVersion(1)
    }

    fn supports(&self, metadata: BlockMetadata) -> bool {
        metadata.is_tool_result_plain_text()
    }

    fn transform(
        &self,
        input: &[u8],
        limits: &CompressionLimits,
    ) -> Result<Transform, TransformError> {
        let lines = validated_line_values(input, limits)?;
        let mut candidate = b"Tracepress log-prefix-fold v1\n".to_vec();
        let mut index = 0_usize;
        let mut factored = false;
        while index < lines.len() {
            let line = lines.get(index).ok_or(TransformError::Internal)?;
            let Some(prefix) = log_prefix(line) else {
                candidate.extend_from_slice(b"line=");
                write_json_string(&mut candidate, line)?;
                candidate.push(b'\n');
                index = index.saturating_add(1);
                continue;
            };
            let mut end = index.saturating_add(1);
            while let Some(next) = lines.get(end) {
                if log_prefix(next).is_none_or(|next_prefix| next_prefix.value != prefix.value) {
                    break;
                }
                end = end.saturating_add(1);
            }
            if end.saturating_sub(index) >= 2 {
                factored = true;
                candidate.extend_from_slice(b"prefix=");
                write_json_string(&mut candidate, &prefix.value)?;
                candidate.extend_from_slice(b" tails=");
                let tails = lines
                    .get(index..end)
                    .ok_or(TransformError::Internal)?
                    .iter()
                    .map(|line| {
                        let details = log_prefix(line).ok_or(TransformError::Internal)?;
                        let mut tail = String::new();
                        tail.push_str(&line[..details.token_start]);
                        tail.push_str(&line[details.token_end..]);
                        Ok(Value::String(tail))
                    })
                    .collect::<Result<Vec<_>, TransformError>>()?;
                let encoded = serde_json::to_vec(&tails).map_err(|_| TransformError::Internal)?;
                candidate.extend_from_slice(&encoded);
                candidate.push(b'\n');
                index = end;
            } else {
                candidate.extend_from_slice(b"line=");
                write_json_string(&mut candidate, line)?;
                candidate.push(b'\n');
                index = index.saturating_add(1);
            }
            if u64::try_from(candidate.len()).unwrap_or(u64::MAX)
                > limits.max_candidate_output_bytes
            {
                return Err(TransformError::ResourceLimit);
            }
        }
        if !factored {
            return Err(TransformError::NotApplicable);
        }
        readable_text_bounds(&candidate, input, limits)?;
        Ok(Transform::new(candidate, bounded_vec(input, limits)?))
    }

    fn recover(
        &self,
        candidate: &[u8],
        recovery: &[u8],
        limits: &CompressionLimits,
    ) -> Result<Box<[u8]>, TransformError> {
        recover_readable_text(
            candidate,
            recovery,
            b"Tracepress log-prefix-fold v1\n",
            limits,
        )
    }
}

struct BlockRecord {
    repetitions: usize,
    lines: Vec<String>,
}

fn validated_lines<'input>(
    input: &'input [u8],
    limits: &CompressionLimits,
) -> Result<Vec<(&'input [u8], usize)>, TransformError> {
    validate_text(input, limits)?;
    let raw = lines(input);
    let mut result = Vec::new();
    let mut index = 0_usize;
    while index < raw.len() {
        let line = *raw.get(index).ok_or(TransformError::Internal)?;
        let mut count = 1_usize;
        while raw.get(index.saturating_add(count)).copied() == Some(line) {
            count = count.saturating_add(1);
        }
        result.push((line, count));
        index = index.saturating_add(count);
    }
    Ok(result)
}

fn validated_line_values(
    input: &[u8],
    limits: &CompressionLimits,
) -> Result<Vec<String>, TransformError> {
    validate_text(input, limits)?;
    lines(input)
        .into_iter()
        .map(|line| {
            std::str::from_utf8(line)
                .map(str::to_owned)
                .map_err(|_| TransformError::InvalidInput)
        })
        .collect()
}

fn repeated_block_records(
    lines: &[String],
    limits: &CompressionLimits,
) -> Result<Vec<BlockRecord>, TransformError> {
    let mut records = Vec::new();
    let mut index = 0_usize;
    let mut work = 0_u64;
    while index < lines.len() {
        let remaining = lines.len().saturating_sub(index);
        let mut best = None::<(usize, usize)>;
        for pattern_len in 2..=remaining.saturating_div(2).min(8) {
            work = work.saturating_add(1);
            if work > limits.max_shadow_work_units {
                return Err(TransformError::ResourceLimit);
            }
            let pattern = lines
                .get(index..index.saturating_add(pattern_len))
                .ok_or(TransformError::Internal)?;
            let mut repetitions = 1_usize;
            while lines.get(
                index.saturating_add(repetitions.saturating_mul(pattern_len))
                    ..index
                        .saturating_add(repetitions.saturating_add(1).saturating_mul(pattern_len)),
            ) == Some(pattern)
            {
                repetitions = repetitions.saturating_add(1);
                work = work.saturating_add(u64::try_from(pattern_len).unwrap_or(u64::MAX));
                if work > limits.max_shadow_work_units {
                    return Err(TransformError::ResourceLimit);
                }
            }
            if repetitions >= 2
                && best.is_none_or(|(best_len, best_repetitions)| {
                    pattern_len.saturating_mul(repetitions)
                        > best_len.saturating_mul(best_repetitions)
                })
            {
                best = Some((pattern_len, repetitions));
            }
        }
        if let Some((pattern_len, repetitions)) = best {
            let pattern = lines
                .get(index..index.saturating_add(pattern_len))
                .ok_or(TransformError::Internal)?
                .to_vec();
            records.push(BlockRecord {
                repetitions,
                lines: pattern,
            });
            index = index.saturating_add(pattern_len.saturating_mul(repetitions));
        } else {
            records.push(BlockRecord {
                repetitions: 1,
                lines: vec![lines.get(index).ok_or(TransformError::Internal)?.clone()],
            });
            index = index.saturating_add(1);
        }
    }
    Ok(records)
}

struct LogPrefix {
    value: String,
    token_start: usize,
    token_end: usize,
}

fn log_prefix(line: &str) -> Option<LogPrefix> {
    let mut token_count = 0_u8;
    let mut in_token = false;
    let mut second_token_start = None;
    let mut second_token_end = None;
    let mut prefix_end = None;
    for (index, byte) in line.bytes().enumerate() {
        if byte.is_ascii_whitespace() {
            if in_token {
                token_count = token_count.saturating_add(1);
                in_token = false;
                if token_count == 2 {
                    second_token_end = Some(index);
                    prefix_end = Some(index.saturating_add(1));
                    break;
                }
            }
        } else {
            if token_count == 1 && !in_token {
                second_token_start = Some(index);
            }
            in_token = true;
        }
    }
    let token_start = second_token_start?;
    let token_end = second_token_end?;
    let prefix_end = prefix_end.unwrap_or(line.len());
    let value = line.get(token_start..prefix_end)?.to_owned();
    (value.len() >= 3).then_some(LogPrefix {
        value,
        token_start,
        token_end,
    })
}

fn write_json_string(output: &mut Vec<u8>, value: &str) -> Result<(), TransformError> {
    let encoded = serde_json::to_vec(value).map_err(|_| TransformError::Internal)?;
    output.extend_from_slice(&encoded);
    Ok(())
}

fn readable_text_bounds(
    candidate: &[u8],
    input: &[u8],
    limits: &CompressionLimits,
) -> Result<(), TransformError> {
    if std::str::from_utf8(candidate).is_err()
        || u64::try_from(candidate.len()).unwrap_or(u64::MAX) > limits.max_candidate_output_bytes
        || u64::try_from(candidate.len().saturating_add(input.len())).unwrap_or(u64::MAX)
            > limits.max_shadow_memory_bytes
    {
        Err(TransformError::ResourceLimit)
    } else {
        Ok(())
    }
}

fn recover_readable_text(
    candidate: &[u8],
    recovery: &[u8],
    prefix: &[u8],
    limits: &CompressionLimits,
) -> Result<Box<[u8]>, TransformError> {
    validate_recovery_inputs(candidate, recovery, limits)?;
    if !candidate.starts_with(prefix) || std::str::from_utf8(candidate).is_err() {
        return Err(TransformError::InvalidInput);
    }
    Ok(bounded_vec(recovery, limits)?.into_boxed_slice())
}

fn validate_text(input: &[u8], limits: &CompressionLimits) -> Result<(), TransformError> {
    let bytes = u64::try_from(input.len()).unwrap_or(u64::MAX);
    if bytes > limits.max_shadow_work_units {
        return Err(TransformError::ResourceLimit);
    }
    std::str::from_utf8(input)
        .map(|_text| ())
        .map_err(|_| TransformError::InvalidInput)
}

fn lines(input: &[u8]) -> Vec<&[u8]> {
    let mut result = Vec::new();
    let mut start = 0_usize;
    for (index, byte) in input.iter().enumerate() {
        if *byte == b'\n' {
            let end = index.saturating_add(1);
            if let Some(line) = input.get(start..end) {
                result.push(line);
            }
            start = end;
        }
    }
    if start < input.len() {
        if let Some(line) = input.get(start..) {
            result.push(line);
        }
    }
    result
}

fn encode_records(
    magic: [u8; 4],
    records: &[(usize, Vec<&[u8]>)],
    limits: &CompressionLimits,
) -> Result<Transform, TransformError> {
    let mut output = magic.to_vec();
    write_varint(
        &mut output,
        u64::try_from(records.len()).unwrap_or(u64::MAX),
    );
    for (repetitions, pattern) in records {
        write_varint(&mut output, u64::try_from(*repetitions).unwrap_or(u64::MAX));
        write_varint(
            &mut output,
            u64::try_from(pattern.len()).unwrap_or(u64::MAX),
        );
        for line in pattern {
            write_varint(&mut output, u64::try_from(line.len()).unwrap_or(u64::MAX));
            output.extend_from_slice(line);
            if u64::try_from(output.len()).unwrap_or(u64::MAX) > limits.max_candidate_output_bytes {
                return Err(TransformError::ResourceLimit);
            }
        }
    }
    Ok(Transform::new(output, Vec::new()))
}

fn decode_records(
    input: &[u8],
    magic: [u8; 4],
    limits: &CompressionLimits,
) -> Result<Box<[u8]>, TransformError> {
    validate_recovery_inputs(input, &[], limits)?;
    if input.get(..magic.len()) != Some(magic.as_slice()) {
        return Err(TransformError::InvalidInput);
    }
    let mut cursor = magic.len();
    let records = read_usize(input, &mut cursor)?;
    let mut output = Vec::new();
    let mut work = 0_u64;
    for _ in 0..records {
        let repetitions = read_usize(input, &mut cursor)?;
        let pattern_length = read_usize(input, &mut cursor)?;
        if pattern_length > input.len()
            || u64::try_from(pattern_length).unwrap_or(u64::MAX) > limits.max_shadow_work_units
        {
            return Err(TransformError::ResourceLimit);
        }
        let mut pattern = Vec::new();
        pattern
            .try_reserve(pattern_length)
            .map_err(|_| TransformError::ResourceLimit)?;
        for _ in 0..pattern_length {
            let length = read_usize(input, &mut cursor)?;
            let end = cursor
                .checked_add(length)
                .ok_or(TransformError::ResourceLimit)?;
            let line = input.get(cursor..end).ok_or(TransformError::InvalidInput)?;
            cursor = end;
            pattern.push(line);
        }
        for _ in 0..repetitions {
            for line in &pattern {
                let projected = output
                    .len()
                    .checked_add(line.len())
                    .ok_or(TransformError::ResourceLimit)?;
                if u64::try_from(projected).unwrap_or(u64::MAX) > limits.max_shadow_memory_bytes {
                    return Err(TransformError::ResourceLimit);
                }
                output.extend_from_slice(line);
                work = work.saturating_add(1);
                if work > limits.max_shadow_work_units {
                    return Err(TransformError::ResourceLimit);
                }
            }
        }
    }
    if cursor != input.len() {
        return Err(TransformError::InvalidInput);
    }
    Ok(output.into_boxed_slice())
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

    fn text_metadata() -> BlockMetadata {
        BlockMetadata {
            origin: BlockOrigin::ToolGenerated,
            kind: BlockKind::ToolResult,
            detected_kind: DetectedKind::PlainText,
            input_estimated_tokens: Some(1_000),
            request_offset: 200,
            request_analysis_bytes: 2_000,
            persistence: Some(3),
            exact_repetition: Some(true),
        }
    }

    fn json_metadata() -> BlockMetadata {
        BlockMetadata {
            detected_kind: DetectedKind::Json,
            ..text_metadata()
        }
    }

    #[test]
    fn repeated_line_and_run_are_exactly_recoverable() {
        let line_input = b"a long repeated output line\na long repeated output line\na long repeated output line\na long repeated output line\n";
        let line = evaluate(
            &TextRepeatedLine,
            text_metadata(),
            line_input,
            &CompressionLimits::default(),
        );
        assert_eq!(line.metrics().status, CandidateStatus::Applicable);
        assert!(line.metrics().recovery_verified);

        let run_input = b"alpha long line\nbeta long line\ngamma long line\nalpha long line\nbeta long line\ngamma long line\nalpha long line\nbeta long line\ngamma long line\n";
        let run = evaluate(
            &TextRepeatedRun,
            text_metadata(),
            run_input,
            &CompressionLimits::default(),
        );
        assert_eq!(run.metrics().status, CandidateStatus::Applicable);
        assert!(run.metrics().recovery_verified);
        assert!(run.metrics().deterministic);
    }

    #[test]
    fn text_candidates_reject_invalid_utf8_and_json_metadata() {
        let invalid = evaluate(
            &TextRepeatedLine,
            text_metadata(),
            &[0xff, 0xfe],
            &CompressionLimits::default(),
        );
        assert_eq!(invalid.metrics().status, CandidateStatus::InvalidInput);
        let wrong_target = evaluate(
            &TextRepeatedLine,
            json_metadata(),
            b"line\nline\n",
            &CompressionLimits::default(),
        );
        assert_eq!(
            wrong_target.metrics().status,
            CandidateStatus::NotApplicable
        );
    }

    #[test]
    fn decompression_expansion_is_bounded() {
        let limits = CompressionLimits {
            max_shadow_memory_bytes: 8,
            ..CompressionLimits::default()
        };
        let result = evaluate(
            &TextRepeatedLine,
            text_metadata(),
            b"long line\nlong line\nlong line\n",
            &limits,
        );
        assert_eq!(result.metrics().status, CandidateStatus::ResourceLimit);
    }

    #[test]
    fn recovery_rejects_hostile_pattern_capacity_before_allocation() {
        let mut candidate = b"TPR2".to_vec();
        write_varint(&mut candidate, 1);
        write_varint(&mut candidate, 1);
        write_varint(&mut candidate, u64::MAX);
        let limits = CompressionLimits {
            max_candidate_output_bytes: 64,
            max_shadow_memory_bytes: 64,
            ..CompressionLimits::default()
        };
        assert_eq!(
            TextRepeatedRun.recover(&candidate, &[], &limits),
            Err(TransformError::ResourceLimit)
        );
    }

    #[test]
    fn recovery_rejects_overflowing_varints() {
        let mut candidate = b"TPR2".to_vec();
        candidate.extend([0x80; 9]);
        candidate.push(0x02);
        assert_eq!(
            TextRepeatedRun.recover(&candidate, &[], &CompressionLimits::default()),
            Err(TransformError::InvalidInput)
        );
    }

    #[test]
    fn readable_text_candidates_are_explicit_and_recoverable() {
        let repeated = b"warning foo\nwarning foo\nwarning foo\nunique\n";
        let line = evaluate(
            &TextReadableLineFold,
            text_metadata(),
            repeated,
            &CompressionLimits::default(),
        );
        assert!(line.metrics().recovery_verified);
        assert!(line.metrics().deterministic);
        assert!(
            line.payload()
                .is_some_and(|bytes| bytes.starts_with(b"Tracepress line-fold v1\n"))
        );

        let block = b"A\nB\nA\nB\nA\nB\n";
        let folded = evaluate(
            &TextReadableBlockFold,
            text_metadata(),
            block,
            &CompressionLimits::default(),
        );
        assert!(folded.metrics().recovery_verified);
        assert!(folded.metrics().deterministic);

        let logs = b"12:00 INFO worker=1 ok\n12:01 INFO worker=2 ok\n12:02 INFO worker=3 ok\n";
        assert_eq!(
            log_prefix("12:00 INFO worker=1 ok\n").map(|prefix| prefix.value),
            Some("INFO ".to_owned())
        );
        let prefix = evaluate(
            &TextLogPrefixFold,
            text_metadata(),
            logs,
            &CompressionLimits::default(),
        );
        assert!(prefix.metrics().recovery_verified);
        assert!(prefix.metrics().deterministic);
    }
}
