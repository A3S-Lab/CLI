//! Shared Runtime-use directives for agent-driven TUI workflows.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RuntimePolicy {
    /// OS Runtime evidence is required before the final answer.
    Required,
    /// OS Runtime is preferred when it adds useful fan-out, but local fallback is OK.
    Preferred,
    /// The workflow intentionally stays local.
    LocalOnly,
}

impl RuntimePolicy {
    pub(crate) fn directive(self) -> &'static str {
        match self {
            RuntimePolicy::Required => {
                "Runtime evidence is required before the final answer: call the signed-in \
                 OS A3S Runtime through `runtime`, dispatch independent branches with \
                 `parallel_task`, or execute an OS progressive operation with \
                 `\"shaped\":true` / `shaped:true` that returns `.view` or `viewUrl`. \
                 If this cannot be done, explicitly explain why and do not claim Runtime \
                 or RemoteUI success."
            }
            RuntimePolicy::Preferred => {
                "Runtime preference: when OS is signed in and the work has independent \
                 branches, prefer OS A3S Runtime `runtime` tasks or `parallel_task` \
                 fan-out before local serial inspection. If local work is enough or \
                 Runtime is unavailable, state that fallback explicitly."
            }
            RuntimePolicy::LocalOnly => {
                "Local-only runtime policy: stay local; do not call OS Runtime, open \
                 RemoteUI/WebIDE/browser pages, or claim `.view`/`viewUrl` output."
            }
        }
    }
}
