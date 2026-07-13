---
name: debug
description: Deep debugging and root cause analysis
when_to_use: Facing a bug, crash, or unexpected behavior
argument_hint: <error-or-description>
---

# Debug & Root Cause Analysis

## Process
1. **Reproduce** — Create minimal reproduction case
2. **Isolate** — Binary search to find the exact cause
3. **Diagnose** — Check logs, stack traces, state dumps
4. **Fix** — Implement and validate the fix
5. **Verify** — Ensure no regressions

## Techniques
- Add targeted logging/tracing
- Check recent changes with git bisect
- Verify assumptions with small experiments
- Check for common patterns (null, race, overflow, leak)
