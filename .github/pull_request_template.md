## What and why

<!-- What this pull request changes and why. Link the issue: Fixes #123 -->

## How it was tested

<!-- Commands run, on which distribution and Plasma version. Never on a write path to a real keyboard without saying so. -->

## Checklist

- [ ] `scripts/ci-local.sh --fast` passes (fmt, clippy `-D warnings`, tests)
- [ ] Tests added or updated for the changed behaviour
- [ ] User-visible strings go through `tr!` / `i18n()` and the catalogs are refreshed (CONTRIBUTING.md, Translations)
- [ ] Documentation updated (`docs/`, and `CHANGELOG.md` when user-visible)
- [ ] No new write to the keyboard outside the named operations of `akm-core`
