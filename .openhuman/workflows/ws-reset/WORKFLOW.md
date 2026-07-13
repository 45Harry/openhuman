---
name: ws-reset
description: Reset the current branch to main, fetch upstream, and update submodules
allowed-tools: [Bash]
when_to_use: Need to reset workspace to clean main state
---

# ws-reset

Reset workspace to a clean state. Performs:
1. Check for dirty working tree (unless --force)
2. Fetch upstream
3. Checkout main
4. Hard reset to upstream/main
5. Update submodules
6. Report git status

## Usage
Run this skill when you need to discard local changes and sync to the latest upstream main.

## Safety
- Refuses to run if working tree has uncommitted changes (unless `--force`)
- Never pushes
- Only affects local branch state
