# Taste (Continuously Learned by [CommandCode][cmd])

[cmd]: https://commandcode.ai/

# rust

- Pass `root` (or equivalent context) as an explicit parameter through every function in a call chain rather than relying on global/env state. Confidence: 0.85
- When a function needs configuration options, create a `_with_options` variant that accepts an options struct rather than adding parameters to the original function. Confidence: 0.75
- Resolve and canonicalize paths at the earliest point in a function, then operate on the resolved path for all subsequent checks. Confidence: 0.70
- Format method chains with each method call on its own indented line, breaking after the receiver expression. Confidence: 0.70
- Consolidate related imports from the same module into a single grouped import statement rather than maintaining separate import lines. Confidence: 0.65

# ci-workflow

- Add `concurrency` groups with `cancel-in-progress: true` to GitHub Actions workflow jobs to prevent duplicate runs on the same PR. Confidence: 0.75
- Set `fetch-depth: 0` in checkout actions for PR review workflows to ensure full git history is available. Exception: `.github/workflows/claude-code-review.yml` uses `fetch-depth: 1` (lightweight review via Claude Code action doesn't require full history). Confidence: 0.65

# error-handling

- Use inline `{variable}` interpolation in `bail!` / `format!` macros for simpler error messages, and fall back to positional `{}` arguments only when the format string itself is dynamically constructed. Confidence: 0.60
