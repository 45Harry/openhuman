---
name: pr-manager-lite
description: Lightweight PR finisher
when_to_use: PR branch is already checked out locally, just need to apply review fixes
allowed-tools: [Bash, Read, Edit, Write, Grep, Glob]
---

# PR Manager Lite

Lightweight PR finishing workflow. Assumes PR branch is already checked out locally.

## Process
1. Collect reviewer comments from GitHub
2. Apply fixes for each comment
3. Run quality suite (lint, typecheck, tests)
4. Commit fixes with conventional commits
5. Push to PR branch
6. Post review response to GitHub

## Guardrails
- Never push to main
- Never force-push
- Never commit secrets
- Stop if working tree dirty
