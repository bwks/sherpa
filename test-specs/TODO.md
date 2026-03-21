# Test Specs — TODO

Items to address as the test suite matures. None are blockers for writing tests now.

---

## Test Infrastructure Documentation

Document how to actually run tests across the project. Currently the db test setup lives in AGENTS.md but there's no central reference covering:

- How to run unit tests vs integration tests (feature flags, `#[ignore]`, `--ignored`)
- External service requirements per crate (Docker, libvirt, SurrealDB)
- Test isolation patterns (e.g., the db crate's namespace-per-test + teardown approach)
- Environment variables needed (`SHERPA_DB_PASSWORD`, etc.)
- CI considerations for tests requiring privileged access or running services

**Where:** Add a `test-specs/infrastructure.md` or similar.

---

## Prioritised Implementation Order

The specs have P0/P1/P2 markers per test case, but no overall guide for where to start. Recommended order based on effort vs value:

1. **validate** — Pure logic, no external deps, gaps already identified, fast to write
2. **template** — Pure rendering, no external deps, high coverage payoff across 25+ vendors
3. **topology** — Pure parsing, no external deps, small surface area
4. **shared** — Utilities are mostly pure logic, many already partially tested
5. **db** — Existing 28 tests as a pattern to follow, relationship/schema gaps to fill
6. **server/services** — Highest risk area, most complex, but requires integration test infrastructure
7. **container/libvirt/network** — Require running services, best tackled once integration patterns are established
8. **client** — Mostly integration tests requiring a running server
9. **integration/** — E2E tests, last priority, require full stack running

**Where:** Could live in this file or in the README.

---

## Test Data and Fixtures

As tests grow, shared test helpers and fixtures will emerge. Document conventions for:

- Where shared test helpers live (per-crate `tests/helper.rs` vs workspace-level)
- How to construct test data (builder patterns, factory functions)
- Naming conventions for test functions
- Cleanup/teardown patterns for integration tests
- Mock vs real service boundaries

**Where:** Add a `test-specs/fixtures.md` once patterns solidify from the first few crates.
