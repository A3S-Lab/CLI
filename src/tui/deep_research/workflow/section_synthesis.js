async function run(ctx, inputs) {
  const input = inputs.input || {};
  const sections = Array.isArray(input.sections) ? input.sections : [];
  if (inputs.kind === "workflow") {
    const outputs = inputs.step_outputs || {};
    const failures = inputs.step_failures || {};
    const pending = sections.filter((section) => !outputs[section.step_id] && !failures[section.step_id]);
    if (pending.length > 0) {
      return {
        type: "schedule_steps",
        steps: pending.map((section) => ({
          step_id: section.step_id,
          step_name: "generate_section",
          input: section.generation_args,
          retry: { max_attempts: 2, delay_ms: 0 },
        })),
      };
    }
    const failed = sections.filter((section) => failures[section.step_id]);
    if (failed.length > 0) {
      return {
        type: "fail",
        error: `section generation failed for ${failed.map((section) => section.section_id).join(", ")}`,
      };
    }
    return {
      type: "complete",
      output: {
        sections: sections.map((section) => ({
          section_id: section.section_id,
          result: outputs[section.step_id],
        })),
      },
    };
  }
  if (inputs.kind === "step" && inputs.step_name === "generate_section") {
    const result = await ctx.tool("generate_object", inputs.input);
    const exitCode = Number(result && (result.exitCode ?? result.exit_code));
    if (!result || exitCode !== 0) {
      const detail = result && typeof result.output === "string"
        ? result.output
        : "generate_object returned no structured tool result";
      throw new Error(`Report structured generation failed: ${detail}`);
    }
    return result;
  }
  return { error: "unknown section synthesis invocation" };
}
