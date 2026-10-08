-- Rebind only the host-owned turn trace to its scoped protocol projection.
-- Rust installs replacement triggers in the same migration transaction.
DROP TRIGGER IF EXISTS privacy_chat_turn_trace_INSERT;
DROP TRIGGER IF EXISTS privacy_chat_turn_trace_UPDATE;
