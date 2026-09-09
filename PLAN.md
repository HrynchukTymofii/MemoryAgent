# Strip Claude attribution trailers from history and make the rule permanent

- (history) — rewrite the 21 commits `e261894..HEAD`, removing every
  `Co-Authored-By: Claude` and `Claude-Session:` line
- CLAUDE.md — add a "Commits" section forbidding those trailers
- memory/git-commit-policy.md — extend to cover `Claude-Session:` too
