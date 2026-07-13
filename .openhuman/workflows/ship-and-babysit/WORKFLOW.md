---
name: ship-and-babysit
description: Commit, push to fork, open PR, then babysit CI and CodeRabbit until green
allowed-tools: [Bash, Read, Edit, Write]
when_to_use: After completing feature work, ready to ship a PR
triggers: []
---

# Ship & Babysit

A 4-phase workflow to ship code:

## Phase 1: Commit
- Stage changed files
- Create conventional commit (feat/fix/refactor/chore/docs/test)

## Phase 2: Push
- Push to origin fork (never upstream)
- Verify push succeeded

## Phase 3: Open PR
- Create PR against upstream main using `gh pr create`
- Set title, body, and labels

## Phase 4: Babysit (polling loop)
- Poll every ~5 min for CI status and CodeRabbit review comments
- Fix CI failures
- Apply CodeRabbit suggestions
- Resolve review threads
- Exit when all checks green and all threads resolved

## Guardrails
- Never push to main
- Never force-push
- Never commit secrets
- Stop if working tree dirty
- Max 12 polling iterations (~1 hour)
