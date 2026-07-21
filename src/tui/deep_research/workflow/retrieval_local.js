  const localRetrievalSchema = {
    type: "object",
    additionalProperties: false,
    properties: {
      sources: {
        type: "array",
        minItems: 1,
        maxItems: MAX_LOCAL_SOURCES,
        items: {
          type: "object",
          additionalProperties: false,
          properties: {
            url_or_path: { type: "string", minLength: 1, maxLength: 1200 },
            ranges: {
              type: "array",
              minItems: 1,
              maxItems: MAX_LOCAL_RANGES,
              items: {
                type: "object",
                additionalProperties: false,
                properties: {
                  offset: {
                    type: "integer",
                    minimum: 0,
                    maximum: 1000000,
                  },
                  limit: {
                    type: "integer",
                    minimum: 1,
                    maximum: MAX_LOCAL_RANGE_LINES,
                  },
                },
                required: ["offset", "limit"],
              },
            },
          },
          required: ["url_or_path", "ranges"],
        },
      },
    },
    required: ["sources"],
  };
  const normalizeLocalPath = (value) => String(value || "")
    .trim()
    .replace(/\\/g, "/")
    .replace(/^\.\//, "")
    .replace(/\/+/g, "/");
  const observedLocalAnchor = (reported, anchors) => {
    const candidate = normalizeLocalPath(reported);
    if (!candidate) {
      return "";
    }
    for (const anchor of anchors) {
      const observed = normalizeLocalPath(anchor.url_or_path);
      if (candidate === observed) {
        return observed;
      }
    }
    return "";
  };
  const cleanLocalReadText = (value) => String(value || "")
    .replace(/\r\n?/g, "\n")
    .split("\n")
    .filter((line) =>
      !/^\s*\.\.\. \(more lines available; continue with offset=\d+\)\s*$/.test(line)
    )
    .map((line) => line.replace(/^\s*\d+\t/, ""))
    .join("\n")
    .trim();
