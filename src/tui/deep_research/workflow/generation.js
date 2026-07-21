async function run(ctx, inputs) {
  const stepId = "generation";
  if (inputs.kind === "workflow") {
    const outputs = inputs.step_outputs || {};
    const failures = inputs.step_failures || {};
    if (outputs[stepId]) {
      return {
        type: "complete",
        output: { result: outputs[stepId] },
      };
    }
    if (failures[stepId]) {
      return {
        type: "fail",
        error: failures[stepId].error || "durable structured generation failed",
      };
    }
    const requestedAttempts = Number(
      inputs.input && inputs.input.max_attempts || 1
    );
    const maxAttempts = Number.isFinite(requestedAttempts)
      ? Math.max(1, Math.min(2, Math.floor(requestedAttempts)))
      : 1;
    return {
      type: "schedule_step",
      step_id: stepId,
      step_name: "generate_object",
      input: inputs.input && inputs.input.generation_args
        ? inputs.input.generation_args
        : {},
      retry: { max_attempts: maxAttempts, delay_ms: 0 },
    };
  }
  if (inputs.kind === "step" && inputs.step_name === "generate_object") {
    const result = await ctx.tool("generate_object", inputs.input);
    const exitCode = Number(result && (result.exitCode ?? result.exit_code));
    if (!result || exitCode !== 0) {
      const detail = result && typeof result.output === "string"
        ? result.output
        : "generate_object returned no structured tool result";
      throw new Error(`Durable structured generation failed: ${detail}`);
    }
    return result;
  }
  return { error: "unknown durable generation invocation" };
}
