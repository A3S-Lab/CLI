"""JSON-lines Sentence Transformers worker for the ignored CLI evaluation."""

from __future__ import annotations

import argparse
import importlib.metadata
import json
import platform
import sys
from typing import Any


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--local-files-only", action="store_true")
    return parser.parse_args()


def main() -> None:
    sys.stdin.reconfigure(encoding="utf-8", errors="strict")
    sys.stdout.reconfigure(encoding="utf-8", errors="strict")
    sys.stderr.reconfigure(encoding="utf-8", errors="strict")
    arguments = parse_args()
    try:
        from sentence_transformers import SentenceTransformer
    except ImportError as error:
        raise SystemExit(
            "install sentence-transformers to run the real embedding evaluation"
        ) from error

    model = SentenceTransformer(
        arguments.model,
        revision=arguments.revision,
        local_files_only=arguments.local_files_only,
    )
    dimension = model.get_sentence_embedding_dimension()
    if not isinstance(dimension, int) or dimension <= 0:
        raise RuntimeError(f"invalid embedding dimension: {dimension}")
    print(
        json.dumps(
            {
                "ready": True,
                "dimension": dimension,
                "pythonVersion": platform.python_version(),
                "sentenceTransformersVersion": importlib.metadata.version(
                    "sentence-transformers"
                ),
                "transformersVersion": importlib.metadata.version("transformers"),
                "torchVersion": importlib.metadata.version("torch"),
                "device": str(model.device),
            },
            separators=(",", ":"),
        ),
        flush=True,
    )

    for line in sys.stdin:
        request: dict[str, Any] = json.loads(line)
        texts = request.get("texts")
        if not isinstance(texts, list) or not texts or not all(
            isinstance(text, str) for text in texts
        ):
            print(
                json.dumps({"error": "texts must be a non-empty string array"}),
                flush=True,
            )
            continue
        try:
            vectors = model.encode(
                texts,
                normalize_embeddings=True,
                convert_to_numpy=True,
                show_progress_bar=False,
            )
        except Exception as error:  # noqa: BLE001 - keep source text out of diagnostics
            print(
                json.dumps(
                    {
                        "error": "embedding failed",
                        "kind": type(error).__name__,
                    },
                    separators=(",", ":"),
                ),
                flush=True,
            )
            continue
        if len(vectors) != len(texts):
            print(json.dumps({"error": "embedding count mismatch"}), flush=True)
            continue
        print(
            json.dumps(
                {"vectors": [vector.tolist() for vector in vectors]},
                separators=(",", ":"),
            ),
            flush=True,
        )


if __name__ == "__main__":
    main()
