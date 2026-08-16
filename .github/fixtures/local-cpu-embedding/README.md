# Local CPU embedding CI fixture

CI provisions the five files named by model.acl from the immutable
Xenova/all-MiniLM-L6-v2 revision recorded in that manifest. The quantized
ONNX file is about 23 MB, uses mean pooling, produces 384-dimensional vectors,
and is licensed under Apache-2.0.

The model bytes are intentionally not stored in this repository. The
provisioning script downloads each file over HTTPS before the test, verifies
the locked SHA-256 digest, and then the A3S admission path independently
verifies the same manifest with networking disabled for inference.

This fixture is only a cross-platform runtime and lifecycle oracle. The
revision-locked multilingual model and DeepSeek task corpus remain the
retrieval-quality qualification oracle.
