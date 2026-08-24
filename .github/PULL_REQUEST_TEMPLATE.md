<!-- Thank you for the patch. DocBunker parses hostile documents, so the
     checklist below is not bureaucracy: it is how this project stays safe. -->

## Summary

<!-- What does this change do, and why? Link issues with "Fixes #123". -->

## Change type

- [ ] Bug fix
- [ ] New feature
- [ ] Documentation
- [ ] Refactor / tooling
- [ ] Security-relevant change (sandbox boundary, protocol, worker, trust model)

## Security review

Does this change affect the security profile described in `docs/threat-model.md`?

- [ ] No
- [ ] Yes — an ADR in `docs/adr/` is included or updated
- [ ] Not sure (maintainers will assess)

## Checklist

- [ ] `cargo fmt --all -- --check` passes
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] `cargo test --workspace` passes, including tests for the change
- [ ] Frontend checks pass if `frontend/` changed: `npm run lint`, `npm run build`
- [ ] Docs updated (`docs/`, `docs/CHANGELOG.md` unreleased section)
- [ ] No secrets committed; no host-side document parsing; no panics on hostile input
