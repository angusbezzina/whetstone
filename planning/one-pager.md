# One-Pager

> ***Your dependencies evolve. Your coding standards should too.***
> 

## The Problem

AI coding agents write code based on rules you give them. But those rules are often written once and infrequently updated. Meanwhile, dependencies ship new versions, deprecate APIs, and publish better patterns. The agents don't know. Your linters don't check. The gap grows silently.

Semantic best practices — "error messages should be actionable," "prefer composition over inheritance" — are even worse. Everyone agrees on them. No tool enforces them.

## What Whetstone Does

You point Whetstone at the sources you trust — dependency documentation, blog posts, migration guides, team style guides, any URL — and it extracts the rules that matter: migration footguns, non-obvious defaults, convention divergence, breaking changes. It detects your dependencies automatically and resolves their docs as a starting point, but you control what sources inform your rules.
From those approved rules, Whetstone produces two things: native tests and lint configs that enforce rules after code is written, and agent context files ([`AGENTS.md`](http://agents.md/), [`CLAUDE.md`](http://claude.md/), `.cursorrules`) that inform AI agents before they write code. When your sources change, Whetstone tells you exactly what's different and proposes specific new rules. Both outputs regenerate together.
It is a codegen tool, not a runtime dependency. It produces pytest, vitest, and cargo test files that run with your existing CI. A teammate who never installs Whetstone still gets every rule enforced and every agent guided.

## How It Works

```
Init       → Detects your deps, resolves their docs, you add any other sources you trust
Extract    → LLM reads your sources, proposes high-confidence rules, you approve each one
Generate   → Produces native tests, lint configs, and agent context files
Status     → Tells you when sources change and what specific rules to add
Update     → Re-extracts only what changed, you approve, everything regenerates
```

## What Makes It Different

**Rules are derived from sources you choose.** Not from a generic template or a style guide someone wrote once. You decide what sources matter — dependency docs, blog posts, team standards — and Whetstone extracts testable rules from them.

**High confidence or silence.** If a dependency's docs don't clearly state a best practice, Whetstone doesn't invent one. Five rules you trust completely beats fifty you have to review.

**Inform before, enforce after.** The same rules generate both agent instructions (so agents write correct code from the start) and tests (to catch what slips through). One source of truth, two enforcement points.

**Semantic rules are decomposed.** Each rule is broken into the most deterministic signals possible — AST checks, pattern matching, lint mappings. AI judgment is the backstop for what's left, not the primary enforcement.

**Three layers.** Personal preferences (local only, never committed), project standards (committed, enforced in CI), and team/org defaults (inherited across projects). They cascade like git config.

**It stays current.** Whetstone monitors your sources and nudges you with specific recommendations when things change — not "your rules are stale" but "Pydantic deprecated `schema()`, here's a new rule for `model_json_schema()`, do you want to enforce it?"

## Languages

Python, TypeScript, and Rust. Each with native test generation and native linter integration (ruff, biome, clippy).

## Built In Rust

Single binary, no runtime dependencies, fast startup. Install via `brew install whetstone`, `cargo install whetstone`, or `curl | sh`.

## Future

A shared registry where community-validated rules for popular dependencies are ranked by real usage data — approval rates, violation frequency, retention. Adding FastAPI to your project gives you the top rules instantly, ranked by what thousands of developers have found valuable. Individuals and teams can publish and share their rulesets.

---

*Whetstone sharpens the tools that write your code.*