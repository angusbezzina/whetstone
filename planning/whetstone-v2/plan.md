# Whetstone v2 Plan

> Build Whetstone into a fast, incremental, resumable rule-intelligence system for real repositories and monorepos.

---

## Context

Whetstone's current architecture is strong on the core idea:

- detect dependencies
- resolve documentation
- extract high-confidence rules
- generate enforcement and agent context
- monitor freshness

But real-world testing exposed an important product gap:

- dependency detection is fast enough
- full source resolution can become the bottleneck on larger repos
- users do not get enough value soon enough
- repeated runs should not redo expensive work when dependencies have not changed

The v2 plan addresses that gap directly.

---

## v2 Product Direction

Whetstone v2 should optimize for three things:

1. **Speed to first value**
2. **Incremental recomputation**
3. **Durable, resumable workflow state**

The key change is conceptual:

Whetstone should behave less like a single blocking command and more like a layered pipeline with caching, progress, checkpoints, and partial usability.

---

## Core Design Principles

### 1. Fast first response

`doctor` should quickly return high-value structural information:

- languages
- workspaces/manifests
- runtime vs dev dependency counts
- ranked dependency queue
- already cached vs unresolved counts

This output should be useful even before source resolution finishes.

### 2. Incremental by default

Whetstone should not reprocess the entire dependency graph unless explicitly requested.

Subsequent runs should default to:

- changed manifests only
- changed lockfiles only
- changed dependency versions only
- stale source cache only
- unresolved or failed dependencies only

### 3. Per-dependency checkpoints

Source resolution and extraction readiness should persist at dependency granularity.

One slow or failing dependency should not block all others.

### 4. Repo-agnostic prioritization

Dependency ranking should be generic, not hand-curated by framework.

Rank by signals such as:

- runtime vs dev
- source quality (`llms_full_txt`, `llms_txt`, docs-only)
- workspace reach
- dependency centrality
- freshness/confidence
- extraction readiness

### 5. Durable state model

Users should be able to stop and resume work safely.

Whetstone should persist what it knows and why.

---

## Proposed v2 Workflow

### Phase A — Scan

Goal: produce immediate value.

`doctor` should:

- detect manifests and workspaces
- fingerprint manifests and lockfiles
- compute normalized dependency inventory
- identify what is unchanged from prior runs
- build a ranked dependency queue
- emit a partial report immediately

Output should include:

- total dependencies
- languages and workspaces
- cached vs missing vs stale counts
- top-ranked dependencies to resolve next

### Phase B — Resolve

Goal: resolve sources incrementally and in parallel.

Resolution should:

- work from the ranked queue
- cache results per dependency/version
- checkpoint after each resolved dependency
- tolerate partial failure
- surface extraction-ready dependencies as they become available

Important behavior:

- unchanged dependency/version pairs should use cached source results
- stale cache entries should be refreshed
- failures should remain visible and retryable

### Phase C — Extract

Goal: start rule work as soon as enough information exists.

Extraction should not wait for the full repo.

Instead, Whetstone should support:

- resolved-and-ready subsets
- high-confidence first extraction sets
- batch extraction by readiness tier

### Phase D — Approve and Generate

This phase remains similar to MVP v2, but should consume the new incremental state model.

Generated outputs should remain derived from approved rules only.

### Phase E — Monitor and Refresh

`status` should distinguish clearly between:

- dependency changes
- source staleness
- rule drift
- cache health
- ready-to-extract items
- unresolved items

---

## Data Model Changes

### 1. Manifest fingerprint state

Persist fingerprints for each relevant workspace manifest and lockfile:

- path
- file hash
- last seen timestamp
- workspace identity

Purpose:

- skip detection work when manifests are unchanged
- identify targeted rescans when only some workspaces change

### 2. Normalized dependency inventory

Persist the merged project dependency graph with entries like:

- name
- language
- version spec
- dev/runtime
- source manifests/workspaces
- first seen / last seen
- current state

### 3. Source-resolution cache

Persist per dependency/version:

- docs URL
- llms URL if any
- source type
- content hash
- fetch timestamp
- freshness metadata
- confidence
- resolution errors if any

### 4. Dependency processing state

Each dependency should have an explicit lifecycle such as:

- discovered
- queued
- resolving
- resolved
- extraction_ready
- extracted
- approved
- stale
- failed

### 5. Refresh signals

Track invalidation inputs separately:

- manifest changed
- lockfile changed
- dependency version changed
- source TTL expired
- source content hash changed
- manual force refresh

---

## CLI and UX Changes

### Doctor

`doctor` should become a staged orchestrator.

Desired behavior:

- return initial scan results quickly
- continue resolution work without blocking all value
- support explicit resume/continue semantics
- optionally surface progress in human-readable mode

Potential flags and semantics:

- `--changed-only`
- `--refresh`
- `--resume`
- `--max-deps N`
- `--prioritize runtime`
- `--json`

### Status

`status` should evolve into both a health view and a pipeline-progress view.

It should report:

- dependency snapshot changes
- cache hit/miss counts
- stale sources
- unresolved dependencies
- extraction-ready dependencies
- approved-rule coverage

### New output concepts

Outputs should include buckets like:

- **ready now**
- **resolving**
- **cached**
- **stale**
- **failed**
- **low-value / skipped**

---

## Caching and Incremental Strategy

### First run

- full manifest scan
- full dependency inventory
- ranked queue build
- partial source resolution with checkpointing

### Subsequent runs

Default behavior:

- compare manifest and lockfile fingerprints
- recompute dependency inventory only where needed
- reuse cache for unchanged dependency/version pairs
- refresh stale or explicitly requested sources only

### Refresh policy

Cache invalidation should occur when:

- dependency version changes
- source content hash changes
- freshness TTL expires
- prior resolution failed and user retries
- user runs a force-refresh path

---

## Prioritization Strategy

To stay extensible to any project, Whetstone should use a generic ranking model.

Suggested scoring inputs:

- runtime dependency boost
- appears in multiple workspaces boost
- high-quality source boost (`llms_full_txt` > `llms_txt` > docs-only)
- freshness confidence boost
- central framework or high-fan-in indicator boost
- dev-only penalty
- unresolved/failed retry priority controls

The output should be a transparent ranked queue, not a hidden special-case heuristic list.

---

## Implementation Workstreams

### 1. State and cache foundation

- define on-disk files and schemas
- persist manifest fingerprints
- persist normalized dependency inventory
- persist per-dependency source cache
- define lifecycle states

### 2. Incremental detection

- compare manifest/lockfile fingerprints
- isolate changed workspaces
- avoid full rescan when unnecessary

### 3. Incremental source resolution

- resolve by ranked queue
- checkpoint after each dependency
- support resume and retry
- surface cache hits and misses

### 4. Doctor UX redesign

- emit useful early summary
- show progress and partial results
- support extraction-ready subsets

### 5. Status redesign

- expose cache/process state
- separate dependency drift from doc drift
- recommend next actions based on real state

### 6. Extraction integration

- consume resolved subsets
- allow extraction to start before all dependencies finish
- persist per-dependency extraction readiness

### 7. Docs and adoption flow

- explain staged/incremental workflow clearly
- show first-run vs repeat-run behavior
- document force refresh vs normal refresh

---

## Acceptance Criteria

Whetstone v2 should be considered successful when:

1. `doctor` returns useful scan output quickly on a large monorepo
2. repeated runs avoid re-resolving unchanged dependencies by default
3. source resolution is checkpointed and resumable
4. users can act on resolved subsets without waiting for the full repo
5. `status` clearly communicates cache state, drift state, and extraction readiness
6. the workflow remains repo-agnostic and does not rely on framework-specific product logic

---

## Recommended Sequencing

### Phase 1

- state model and cache files
- manifest fingerprinting
- dependency inventory persistence

### Phase 2

- incremental detection
- per-dependency source cache
- resume/retry mechanics

### Phase 3

- doctor UX redesign for immediate value and progress
- status UX redesign for pipeline state

### Phase 4

- extraction on partial subsets
- docs refresh and adoption guide updates

### Phase 5

- dogfood on large repos like Splinter
- refine ranking, retry, and refresh policies from real usage

---

## Summary

Whetstone v2 should turn the current blocking workflow into a practical pipeline:

- detect fast
- cache aggressively
- rank intelligently
- surface partial value early
- rerun mostly on changes
- resume where it left off

That is the path from a promising concept to a tool that fits real engineering workflows.
