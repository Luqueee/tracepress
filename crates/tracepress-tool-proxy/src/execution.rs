#[cfg(unix)]
use std::os::unix::process::ExitStatusExt as _;
use std::{process::Command, time::Instant};

use tracepress_core::{SourceExecutionId, UuidV7Generator};

use crate::{
    admission::decide,
    model::{RewriteDecision, SourceExecutionMetadata},
};

/// Executes a previously admitted simple command without filtering its bytes.
///
/// # Errors
///
/// Returns an error when the command is not admitted or the real process cannot be executed.
pub fn execute_passthrough(
    command: &str,
    ids: &UuidV7Generator,
) -> Result<(Vec<u8>, Vec<u8>, SourceExecutionMetadata), std::io::Error> {
    let RewriteDecision::Passthrough { family, contract } = decide(command) else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "command is not admitted",
        ));
    };
    let mut words = command.split_ascii_whitespace();
    let Some(program) = words.next() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "empty command",
        ));
    };
    let started = Instant::now();
    let executable = if program == "cargo" {
        std::env::var_os("TRACEPRESS_REAL_CARGO").unwrap_or_else(|| program.into())
    } else {
        program.into()
    };
    let output = Command::new(executable).args(words).output()?;
    let stdout = output.stdout;
    let stderr = output.stderr;
    let raw_stdout_bytes = u64::try_from(stdout.len()).unwrap_or(u64::MAX);
    let raw_stderr_bytes = u64::try_from(stderr.len()).unwrap_or(u64::MAX);
    let metadata = SourceExecutionMetadata {
        source_execution_id: SourceExecutionId::generate(ids),
        command_family: family,
        output_contract: contract,
        raw_stdout_bytes,
        raw_stderr_bytes,
        emitted_bytes: raw_stdout_bytes.saturating_add(raw_stderr_bytes),
        exit_code: output.status.code(),
        #[cfg(unix)]
        termination_signal: output.status.signal(),
        #[cfg(not(unix))]
        termination_signal: None,
        duration: started.elapsed(),
    };
    Ok((stdout, stderr, metadata))
}
