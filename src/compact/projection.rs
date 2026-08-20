use a3s_code_core::Message;

pub(crate) const A3S_COMPACT_ROLE: &str = "a3s_compact";

pub(crate) fn is_compact_message(message: &Message) -> bool {
    message.role == A3S_COMPACT_ROLE
}
