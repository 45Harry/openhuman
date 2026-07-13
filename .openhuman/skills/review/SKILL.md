---
description: Comprehensive code review and security audit
when_to_use: Before committing, merging PRs, or deploying code
argument_hint: <file-or-dir-path>
---

# Code Review & Security Audit

Review the code thoroughly for:
1. **Logic bugs** — off-by-one, race conditions, null pointers, type errors
2. **Security vulnerabilities** — injection, XSS, CSRF, auth bypass, path traversal
3. **Performance issues** — N+1 queries, memory leaks, unnecessary allocations
4. **Style & conventions** — naming, formatting, idiom violations
5. **Edge cases** — empty states, error handling, boundary conditions

## Security Review
- Check for hardcoded secrets, tokens, keys
- Verify input validation and sanitization
- Review authentication/authorization logic
- Check dependency vulnerabilities

## Output Format
- **Critical** (must fix before merge)
- **Major** (should fix before merge)
- **Minor** (nice to have)
- **Suggestion** (future improvement)
