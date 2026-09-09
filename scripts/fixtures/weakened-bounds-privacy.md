# Weakened bounds and privacy probe

## Bounded resource contract

Finite `max_request_body_bytes`, `max_response_body_bytes`, `max_ipc_frame_bytes`, and `max_ipc_queue_items` exist.

## Privacy and redaction boundary

Raw bytes remain local and external behavior is metadata only. Authorization, Bearer, API keys, and cookies are redacted. Never persist complete provider headers. External IDs use HMAC-SHA-256.
