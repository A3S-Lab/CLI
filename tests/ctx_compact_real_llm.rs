//! REAL-LLM test of context tracking + auto-compaction — the machinery behind
//! the TUI's ctx% indicator, fill warnings, and mid-turn auto-compact.
//!
//! Proves, against the actually-configured LLM (`~/.a3s/config.acl`):
//!
//!   1. streaming turns REPORT real usage (`TurnEnd.usage.prompt_tokens > 0`)
//!      — this feeds the TUI's live ctx% + warnings; if a gateway stopped
//!      sending the usage chunk this fails first,
//!   2. auto-compaction TRIGGERS once the prompt crosses the threshold
//!      (`ContextCompacted` with fewer messages after than before),
//!   3. the next round's prompt actually SHRINKS after compaction.
//!
//! The test pins a 200k context window and uses a deliberately low 0.05
//! threshold, so compaction triggers at 10k prompt tokens. Turns carry
//! ~800-token fillers, placing the crossing after roughly a dozen turns.
//!
//! Ignored by default — it hits the network + a real model. Run with:
//!   cargo test --test ctx_compact_real_llm -- --ignored --nocapture

use std::sync::Arc;
use std::time::Duration;

use a3s_code_core::hitl::{ConfirmationPolicy, TimeoutAction};
use a3s_code_core::store::{FileSessionStore, SessionStore};
use a3s_code_core::{AgentEvent, AgentSession, SessionOptions};

const TURN_TIMEOUT: Duration = Duration::from_secs(300);
const TEST_CONTEXT_TOKENS: usize = 200_000;
/// Low on purpose: 0.05 × the pinned 200k window = trigger at 10k prompt tokens.
const TEST_THRESHOLD: f32 = 0.05;
const MAX_TURNS: usize = 25;

/// One streamed turn; returns (last TurnEnd prompt_tokens, compactions seen
/// as (before, after) message counts).
async fn turn(sess: &AgentSession, prompt: &str) -> (usize, Vec<(usize, usize)>) {
    let fut = async {
        let (mut rx, _join) = sess.stream(prompt, None).await.expect("stream start");
        let mut prompt_tokens = 0usize;
        let mut compactions = Vec::new();
        while let Some(ev) = rx.recv().await {
            match ev {
                AgentEvent::TurnEnd { usage, .. } => {
                    if usage.prompt_tokens > 0 {
                        prompt_tokens = usage.prompt_tokens;
                    }
                }
                AgentEvent::ContextCompacted {
                    before_messages,
                    after_messages,
                    percent_before,
                    ..
                } => {
                    eprintln!(
                        "[compact] {before_messages} -> {after_messages} messages at {:.0}%",
                        percent_before * 100.0
                    );
                    compactions.push((before_messages, after_messages));
                }
                AgentEvent::End { .. } => break,
                AgentEvent::Error { message } => panic!("turn errored: {message}"),
                _ => {}
            }
        }
        (prompt_tokens, compactions)
    };
    tokio::time::timeout(TURN_TIMEOUT, fut)
        .await
        .expect("turn timed out")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "hits the real configured LLM over the network"]
async fn context_usage_reports_and_auto_compaction_triggers() {
    let home = std::env::var("HOME").expect("HOME");
    let config = format!("{home}/.a3s/config.acl");
    assert!(
        std::path::Path::new(&config).exists(),
        "no ~/.a3s/config.acl — configure a model first"
    );

    let tmp = std::env::temp_dir().join(format!("a3s-ctx-realllm-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let store: Arc<dyn SessionStore> = Arc::new(FileSessionStore::new(&tmp).await.expect("store"));

    let agent = a3s_code_core::Agent::new(config)
        .await
        .expect("build agent from config.acl");
    let cwd = tmp.to_string_lossy().to_string();
    let sess = agent
        .session(
            cwd,
            Some(
                SessionOptions::new()
                    .with_session_store(store.clone())
                    .with_session_id("ctx-compact")
                    .with_auto_save(true)
                    .with_auto_compact(true)
                    .with_max_context_tokens(TEST_CONTEXT_TOKENS)
                    .with_auto_compact_threshold(TEST_THRESHOLD)
                    .with_confirmation_policy(
                        ConfirmationPolicy::enabled().with_timeout(500, TimeoutAction::Reject),
                    ),
            ),
        )
        .expect("session");

    // ~800 tokens of inert filler per turn (unique per turn so nothing dedups).
    let filler = |i: usize| {
        format!(
            "Background note {i}, for context only — no action needed. {}",
            format!("Entry {i}: the quarterly ledger reconciles against invoice batch {i}. ")
                .repeat(50)
        )
    };

    let mut saw_usage = false;
    let mut peak_prompt = 0usize;
    let mut real_compaction: Option<(usize, usize)> = None;
    let mut post_compact_prompt: Option<usize> = None;

    for i in 1..=MAX_TURNS {
        let msg = format!("{}\nDo not use any tools. Reply with only: OK", filler(i));
        let (prompt_tokens, compactions) = turn(&sess, &msg).await;
        eprintln!("[turn {i:>2}] prompt_tokens={prompt_tokens}");
        if prompt_tokens > 0 {
            saw_usage = true;
        }
        if real_compaction.is_some() && post_compact_prompt.is_none() && prompt_tokens > 0 {
            post_compact_prompt = Some(prompt_tokens);
            break;
        }
        peak_prompt = peak_prompt.max(prompt_tokens);
        // A crossing below the 30-message minimum only prunes (before == after);
        // keep going until a real summarization shrinks the history.
        if let Some(&(b, a)) = compactions.iter().find(|(b, a)| a < b) {
            real_compaction = Some((b, a));
        }
    }

    assert!(
        saw_usage,
        "no TurnEnd ever reported prompt_tokens > 0 — the provider/gateway is \
         not sending streaming usage, so ctx% and auto-compact are blind"
    );
    let (before, after) =
        real_compaction.expect("auto-compaction never shrank the history within the turn budget");
    assert!(after < before, "compaction must reduce messages");
    let post = post_compact_prompt.expect("no turn completed after compaction");
    assert!(
        post < peak_prompt,
        "post-compaction prompt ({post}) should be smaller than the pre-compaction \
         peak ({peak_prompt})"
    );

    eprintln!(
        "\n✅ context tracking + auto-compaction verified against the real LLM:\n   \
         - streaming usage reported (peak prompt {peak_prompt} tokens)\n   \
         - auto-compact fired: {before} -> {after} messages\n   \
         - next prompt shrank to {post} tokens\n"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}
