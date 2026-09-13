//! Plain-text `ToolResult` shadow candidates.

use crate::{
    BlockMetadata, CompressionLimits, CompressorId, CompressorVersion, ShadowCompressor, Transform,
    TransformError, bounded_vec, validate_recovery_inputs,
};

/// P0 infrastructure control.
#[derive(Clone, Copy, Debug, Default)]
pub struct TextNoop;

/// P1 consecutive identical-line folding.
#[derive(Clone, Copy, Debug, Default)]
pub struct TextRepeatedLine;

/// P2 bounded adjacent multi-line run folding.
#[derive(Clone, Copy, Debug, Default)]
pub struct TextRepeatedRun;

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
}
