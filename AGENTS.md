# AGENTS.md

Read **[ENGINEERING.md](ENGINEERING.md)** before starting any task. It contains workspace layout, build commands, architecture rules, and engineering discipline shared across all agents.

For tasks in `dynamapper/`, also read **[docs/agents/dynamapper_architecture.md](docs/agents/dynamapper_architecture.md)**.

---

## Codex Notes

### Phase-based commits

Work in completed, validated phases. A phase is complete when the intended subtask is implemented, validated, and the diff is coherent and minimal. Commit only at phase boundaries, staging only files relevant to the completed phase.

Before committing, verify:
- the implementation matches the original intent,
- no exploration or debug clutter remains,
- no unauthorized formatting or linting was performed,
- the change is minimal and reviewable.

Use concise commit messages describing the completed phase, not the mechanism. Do not commit formatting-only changes, linter-only changes, or unvalidated work.

### Post-change review

After implementing any change, explicitly check:

1. Does it implement the intended behavior?
2. Is the approach consistent with the repository's established patterns?
3. Did I introduce unnecessary complexity?
4. Did I leave behind any exploration or audit clutter?
5. Was it validated with a narrowly scoped check?

If validation fails, diagnose carefully once before retrying. Do not enter a blind fix-and-rerun loop.

### Git

Inspect the working tree before editing when appropriate. Do not revert, reformat, or absorb pre-existing uncommitted user changes. Never use `git reset --hard`, `git clean`, force pushes, or history rewrites unless explicitly instructed.
