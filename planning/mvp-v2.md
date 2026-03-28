# Whetstone MVP v2

> Tighten Whetstone into a practical, trustworthy open-source tool for keeping human and agent-written code aligned with current dependency best practices.

---

## Why a V2 Plan

Whetstone already has a strong foundation:

- dependency detection across Python, TypeScript, and Rust
- source resolution from registries and docs
- rule-schema and generation primitives
- agent-context generation
- status and CI-oriented freshness checks

The gap is not "does the idea matter?" The gap is product sharpness.

In the current agentic coding landscape, there are many strong tools for:

- linting and static analysis
- PR review
- AI-assisted fixing
- workflow orchestration

But there are still relatively few tools that:

1. derive high-confidence rules from the actual documentation of a project's dependencies
2. tie those rules to explicit source URLs and deterministic signals
3. generate both enforcement artifacts and agent instructions from the same approved rules
4. warn when those rules become stale

That is the wedge Whetstone should own.

---

## Product Thesis

Whetstone should be the tool that:

- finds a small number of high-confidence, dependency-specific best-practice rules
- proves them with source links and deterministic signals
- turns them into normal repo artifacts developers already use
- keeps agents informed with the same approved rules
- flags when docs or dependency versions drift

Whetstone should **not** try to be:

- a general AI code reviewer
- a replacement for Ruff, Biome, Clippy, Semgrep, reviewdog, or PR bots
- a broad semantic-eval platform for every subjective coding concern

### Core positioning

Whetstone is the **rule-intelligence layer**.

Other tools execute checks, review pull requests, or apply fixes. Whetstone decides which rules are worth enforcing in the first place, why they matter, and how they map into deterministic enforcement and agent guidance.

---

## Design Principles

### 1. High confidence or silence

Five trusted rules are better than fifty noisy ones.

### 2. Deterministic-first

Every approved rule must have at least one deterministic signal or explicit lint mapping.

### 3. Source-backed by default

Every rule should cite a real documentation URL and preserve enough provenance to be reviewable later.

### 4. Complement existing tooling

Whetstone should integrate with existing enforcement surfaces instead of reinventing them.

- Python: Ruff + pytest
- TypeScript: Biome / typescript-eslint / Semgrep + vitest
- Rust: Clippy / Dylint where appropriate + cargo test

### 5. Useful in a real repo today

The product should feel worthwhile for a solo engineer or small team before it tries to become a larger platform.

---

## MVP v2 Goals

By the end of MVP v2, Whetstone should be:

1. **Understandable**
   - docs and commands clearly distinguish current behavior from future ambitions

2. **Repeatable**
   - the workflow from detection to approved outputs is explicit and reproducible

3. **Trustworthy**
   - rules carry provenance, confidence, and deterministic enforcement paths

4. **Practical across the target languages**
   - Python is strong
   - TypeScript and Rust are narrower but genuinely useful

5. **Comfortable to adopt**
   - users can run Whetstone, understand the results, and keep using it in normal development loops

---

## Non-Goals for MVP v2

The following should be explicitly deprioritized unless they directly improve the core wedge:

- building a general-purpose PR review agent
- trying to auto-fix everything
- broad AI-eval infrastructure for subjective style rules
- registry/community platform work
- expansive layer/promote systems beyond what is needed for practical repo use

---

## Canonical Workflow

Whetstone should present one clear workflow:

1. **Doctor**
   - detect dependencies
   - resolve documentation sources
   - summarize what Whetstone can work on
   - propose the next step

2. **Extract**
   - turn approved sources into candidate rules using the agent skill and a stable contract

3. **Approve**
   - review candidate rules
   - persist decisions and metadata clearly

4. **Generate**
   - create tests, lint overlays, and agent context from approved rules

5. **Status**
   - report freshness, coverage, drift, and recommended next actions

6. **Update**
   - revisit changed sources and propose targeted rule updates

Even if some steps remain partly agent-mediated, the workflow itself must be first-class and consistently documented.

---

## Strategic Workstreams

### 1. Tighten the product contract

Align `README.md`, `planning/product-spec.md`, `planning/mvp.md`, and command docs around the real product wedge:

- dependency-doc-derived rules
- deterministic-first enforcement
- source-backed provenance
- generated agent guidance
- freshness monitoring

Current behavior and planned behavior should be clearly labeled.

### 2. Productize extraction and approval

The current model depends too much on implied operator knowledge.

V2 should define:

- candidate rule contracts
- approval state and persistence
- required metadata per rule
- what the scripts do vs what the agent does

### 3. Harden discovery and source quality

Dependency detection and source resolution need stronger boundaries and trust signals:

- better fixture/test/generated-path exclusion
- clearer monorepo behavior
- source freshness metadata
- more explicit stale-source handling

### 4. Improve enforcement outputs by language

#### Python

Python should remain the reference-quality path.

Focus:

- high-confidence pytest generation
- strong Ruff mappings
- clearer support matrix for signal types

#### TypeScript

TypeScript support should become narrower but more trustworthy.

Focus:

- align with Biome, typescript-eslint, and Semgrep where they are strongest
- reduce placeholder/scaffold output
- be explicit about supported signal classes

#### Rust

Rust support should also become narrower but credible.

Focus:

- strong Clippy alignment
- cargo-test-based checks for selected signals
- evaluate Dylint as the path for richer custom linting

### 5. Improve rule quality and provenance

Every approved rule should remain explainable end-to-end:

- source URL
- source hash or drift marker
- confidence
- category
- deterministic signals
- golden examples
- generated enforcement target

This is central to Whetstone's trust model.

### 6. Build a stronger doctor/status UX

Whetstone needs a more practical user-facing experience inspired by tools like React Doctor, but still aligned with its own wedge.

The UX should answer:

- what did you find?
- what is enforceable now?
- what is stale?
- what should I do next?

### 7. Dogfood and package for actual use

Whetstone should be usable on Whetstone itself and on a small number of representative repos.

This should drive:

- output cleanup
- better defaults
- clearer docs
- sharper limitations

---

## Recommended Sequencing

### Phase 1: Clarify and constrain

- tighten scope and docs
- define the canonical workflow and command contracts

### Phase 2: Make the workflow real

- formalize extraction and approval
- harden dependency and source discovery

### Phase 3: Raise enforcement quality

- strengthen Python as the gold path
- raise TypeScript to a useful baseline
- raise Rust to a useful baseline

### Phase 4: Increase trust and usability

- improve provenance and drift handling
- build a better doctor/status experience

### Phase 5: Prepare for practical open-source use

- dogfood on Whetstone itself
- improve repo hygiene and contributor experience
- publish an OSS-ready adoption guide

---

## Success Criteria

Whetstone MVP v2 succeeds if:

- a new user can understand the product and current scope quickly
- the main workflow is explicit and repeatable
- Python output is strong and TypeScript/Rust outputs are honest and useful
- approved rules are clearly source-backed and explainable
- doctor/status gives actionable recommendations instead of vague summaries
- the tool is credible for day-to-day personal use and reasonable to open source

---

## Proposed Epic

**Epic:** Tighten Whetstone into a practical dependency-best-practices tool

### Child work items

1. Clarify product scope and current-state docs
2. Define the canonical workflow and command contract
3. Productize extraction and approval flow
4. Harden dependency and source discovery
5. Strengthen Python enforcement outputs
6. Raise TypeScript output to a useful baseline
7. Raise Rust output to a useful baseline
8. Improve rule quality model and provenance
9. Build a better doctor/status experience
10. Dogfood Whetstone on Whetstone
11. Polish repo hygiene and contributor experience
12. Publish an OSS-ready adoption guide

This plan intentionally favors trust, focus, and practical utility over platform breadth.
