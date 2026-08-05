use super::*;
use a3s_code_core::CodeIntelligenceError;
use std::future::Future;

const QUERY_DEADLINE: Duration = Duration::from_secs(15);
const PROTOCOL_RETRY_DELAY: Duration = Duration::from_millis(25);

/// Retry one read-only semantic request after a typed protocol failure. Some
/// language servers return a transient content-modified protocol error while a
/// freshly opened saved document is settling. Both attempts share one absolute
/// deadline. The retry is delayed briefly, cancellation-aware, and never
/// classifies human-readable error prose.
pub(super) async fn execute_code_intelligence_with_protocol_retry<T, F, Fut>(
    operation_name: &'static str,
    cancellation: &CancellationToken,
    mut operation: F,
) -> Result<T, CodeIntelligenceError>
where
    F: FnMut(CancellationToken) -> Fut,
    Fut: Future<Output = Result<T, CodeIntelligenceError>>,
{
    let deadline = tokio::time::Instant::now() + QUERY_DEADLINE;
    let attempt_cancellation = cancellation.child_token();
    let result = await_attempt(
        operation_name,
        cancellation,
        attempt_cancellation.clone(),
        deadline,
        operation(attempt_cancellation),
    )
    .await;
    if !matches!(result, Err(CodeIntelligenceError::Protocol { .. })) {
        return result;
    }

    tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Err(CodeIntelligenceError::Cancelled),
        _ = tokio::time::sleep_until(deadline) => {
            return Err(timeout_error(operation_name));
        }
        _ = tokio::time::sleep(PROTOCOL_RETRY_DELAY) => {}
    }

    let attempt_cancellation = cancellation.child_token();
    await_attempt(
        operation_name,
        cancellation,
        attempt_cancellation.clone(),
        deadline,
        operation(attempt_cancellation),
    )
    .await
}

async fn await_attempt<T, Fut>(
    operation_name: &'static str,
    cancellation: &CancellationToken,
    attempt_cancellation: CancellationToken,
    deadline: tokio::time::Instant,
    future: Fut,
) -> Result<T, CodeIntelligenceError>
where
    Fut: Future<Output = Result<T, CodeIntelligenceError>>,
{
    tokio::pin!(future);
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => Err(CodeIntelligenceError::Cancelled),
        _ = tokio::time::sleep_until(deadline) => {
            attempt_cancellation.cancel();
            Err(timeout_error(operation_name))
        }
        result = &mut future => result,
    }
}

fn timeout_error(operation_name: &'static str) -> CodeIntelligenceError {
    CodeIntelligenceError::Timeout {
        operation: operation_name.to_owned(),
        duration: QUERY_DEADLINE,
    }
}
