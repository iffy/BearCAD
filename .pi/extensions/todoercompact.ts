import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";

/**
 * todoercompact — compact context after each completed todoer issue.
 *
 * Watches bash tool results for `todoer done <id>` and triggers compaction
 * so the next issue starts with a mostly-clean context.  The compaction
 * summary preserves what was done (files changed, tests passed) without
 * carrying the full conversation history of the finished issue.
 */
export default function (pi: ExtensionAPI) {
	// Track the last issue we compacted after to avoid double-compacting
	// (the agent may call `todoer done` multiple times for the same issue).
	let lastCompactedIssue: string | undefined;

	pi.on("tool_result", (_event, ctx) => {
		const { call, result } = _event;

		// Only react to bash commands that ran `todoer done`.
		if (call.tool !== "bash" || typeof result !== "string") return;
		if (!result.startsWith("Done ")) return;

		// Extract the issue id, e.g. "Done 1086: ..."
		const match = result.match(/^Done (\S+):/);
		if (!match) return;
		const issueId = match[1];

		// Skip if we already compacted for this issue.
		if (issueId === lastCompactedIssue) return;
		lastCompactedIssue = issueId;

		// Compact with a summary instruction so the compaction preserves
		// what was done and doesn't lose file/tool context the next issue
		// might need.
		ctx.compact({
			customInstructions: [
				"Summarize the completed issue concisely:",
				"- What files were changed",
				"- What the fix/feature was",
				"- Test results",
				"",
				"Preserve any project-wide context (architecture, conventions,",
				"recent refactors) that the next issue might need.  Drop the",
				"detailed conversation of the completed issue.",
			].join("\n"),
			onComplete: () => {
				if (ctx.hasUI) {
					ctx.ui.notify(`Compacted after issue ${issueId}`, "info");
				}
			},
			onError: (error) => {
				if (ctx.hasUI) {
					ctx.ui.notify(`Compaction failed: ${error.message}`, "error");
				}
			},
		});
	});
}