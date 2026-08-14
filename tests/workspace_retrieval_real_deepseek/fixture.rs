use std::fmt::Write as _;
use std::path::Path;

pub(super) const TEST_API_KEY: &str = "HOST_EVAL_KEY_MUST_NOT_LEAK";

pub(super) fn write_fixture(root: &Path) {
    let source = root.join("src");
    std::fs::create_dir_all(&source).expect("create host evaluation source");
    for (path, body) in [
        (
            "replay_fence.rs",
            "pub fn suppress_replayed_envelopes(sequence: u64, committed: u64) -> bool {\n    sequence <= committed\n}\n",
        ),
        (
            "session_projection.rs",
            "pub fn release_ephemeral_projection(generation: &mut Option<u64>) {\n    generation.take();\n}\n",
        ),
        (
            "reconnect_notes.rs",
            "// what routine prevents duplicate delivery after a transport reconnect\npub fn describe_reconnect_incident() {}\n",
        ),
        (
            "cleanup_notes.rs",
            "// 会话结束后，哪个函数负责销毁只存在于内存中的检索投影\npub fn document_shutdown_checklist() {}\n",
        ),
        (
            "queue_notes.rs",
            "// where is the backpressure ceiling for queued embedding work defined\npub const DOCUMENTED_QUEUE_OBSERVATION: usize = 999;\n",
        ),
    ] {
        std::fs::write(source.join(path), body).expect("write host evaluation fixture");
    }
    for index in 0..24 {
        std::fs::write(
            source.join(format!("unrelated_{index:02}.rs")),
            format!(
                "pub fn unrelated_worker_{index:02}(value: usize) -> usize {{ value + {index} }}\n"
            ),
        )
        .expect("write host evaluation distractor");
    }
    let mut chunked_source = String::new();
    for index in 0..90 {
        writeln!(
            chunked_source,
            "// deterministic chunk-boundary filler {index:02}"
        )
        .expect("write host evaluation filler");
    }
    chunked_source.push_str(
        "pub const MAX_PENDING_EMBED_BATCHES: usize = 8;\n\npub fn admits_batch(pending: usize) -> bool {\n    pending < MAX_PENDING_EMBED_BATCHES\n}\n",
    );
    std::fs::write(source.join("embedding_admission.rs"), chunked_source)
        .expect("write host evaluation multi-chunk source");

    let assets = root.join("assets");
    std::fs::create_dir_all(&assets).expect("create host evaluation assets");
    for (path, body) in [
        (
            "architecture.pdf",
            b"%PDF-1.7\nNON_TEXT_ASSET_SENTINEL\n".as_slice(),
        ),
        (
            "slides.pptx",
            b"PK OFFICE NON_TEXT_ASSET_SENTINEL\n".as_slice(),
        ),
        ("recording.mp3", b"ID3 NON_TEXT_ASSET_SENTINEL\n".as_slice()),
    ] {
        std::fs::write(assets.join(path), body).expect("write host evaluation non-text asset");
    }
}

pub(super) fn write_trusted_user_config(home: &Path, base_url: &str) {
    let directory = home.join(".a3s");
    std::fs::create_dir_all(&directory).expect("create host evaluation user config directory");
    let acl = format!(
        r#"providers "host-eval" {{
  apiKey = "{TEST_API_KEY}"
  baseUrl = "{base_url}"
}}

workspace_retrieval {{
  enabled = true
  allow_source_egress = true
  model = "host-eval/embed-v1"
  dimension = 8
  normalization = "unit"
  revision = "acl-host-eval-2026-08-15"
  provider_timeout_ms = 30000
  shutdown_timeout_ms = 5000

  chunking {{
    recursive {{
      target_bytes = 512
      overlap_bytes = 64
      separators = ["\n\n", "\n", ". ", " "]
    }}
  }}

  deterministic_reranker {{
    enabled = true
  }}
}}
"#
    );
    std::fs::write(directory.join("config.acl"), acl)
        .expect("write host evaluation trusted user config");
}
