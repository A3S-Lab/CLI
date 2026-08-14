pub(crate) fn test_config() -> &'static str {
    r#"# preserve-this-comment
default_model = "openai/model-a"
os = "http://127.0.0.1:9"

providers "openai" {
  apiKey = "top-secret-api-key"
  baseUrl = "https://example.com/v1"

  models "model-a" { name = "Model A" }
  models "model-b" { name = "Model B" }
}
"#
}
