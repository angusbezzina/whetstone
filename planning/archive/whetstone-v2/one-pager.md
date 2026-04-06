# Whetstone v2 One-Pager

> Make Whetstone fast, incremental, and trustworthy on real monorepos.

## The Problem

Whetstone's core idea is valuable, but the current workflow still behaves too much like a blocking research task:

- large repos can have dozens of dependencies
- full source resolution can take minutes or time out
- users wait too long before seeing useful results
- repeated runs risk redoing expensive work even when dependencies have not changed

That is acceptable for a prototype, but not for a tool intended for real engineering workflows.

## The Opportunity

Whetstone should become the dependency-best-practices engine that works the way package managers, indexers, and code intelligence tools work:

- scan quickly
- cache aggressively
- prioritize intelligently
- stream partial value early
- resume unfinished work
- refresh only what changed

This keeps Whetstone repo-agnostic. It should not need special handling for specific libraries or hand-tuned project exceptions. Instead, it should rank and process any dependency set using generic signals like source quality, freshness, runtime relevance, and workspace reach.

## Product Thesis

Whetstone v2 is an **incremental rule-intelligence pipeline**.

It should:

- detect dependencies across real repos quickly
- build a ranked queue of dependencies to resolve
- return immediate useful output before full resolution completes
- cache per-dependency source results and dependency snapshots
- re-run primarily on changed dependencies or stale documentation
- let extraction begin on any resolved subset rather than blocking on the full project

## What Changes in v2

### From blocking to staged

Instead of one long-running doctor flow, v2 should support:

1. **Fast scan**
   - detect manifests, workspaces, and dependency inventory
   - show a ranked queue immediately

2. **Incremental resolution**
   - resolve sources in parallel with checkpoints
   - persist results per dependency
   - surface partial results continuously

3. **Ready-to-extract subsets**
   - allow extraction to begin for dependencies already resolved well
   - do not wait for the entire repo to complete

4. **Incremental refresh**
   - only re-run detection/resolution when manifests, lockfiles, dependency versions, or source freshness require it

### From repo-specific thinking to generic prioritization

Whetstone should prioritize dependencies using generic ranking signals:

- runtime over dev dependency
- source quality (`llms_full_txt` > `llms_txt` > docs-only)
- dependency used across multiple workspaces
- freshness and staleness signals
- likely rule richness

### From ephemeral runs to durable state

Whetstone should persist:

- manifest fingerprints by workspace
- normalized dependency inventory
- source-resolution cache per dependency/version
- extraction state per dependency
- stale markers for dependency changes and doc changes

## User Experience Goal

For a large monorepo, the first run should feel like this:

- in seconds: "found 37 runtime deps across Python and TypeScript"
- shortly after: "8 strong candidates are already ready for extraction"
- in background: "resolving the rest; 21 cached, 5 pending, 3 failed"

For subsequent runs, the experience should be:

- "2 manifests changed"
- "3 dependencies changed version"
- "1 documentation source went stale"
- "only those 4 items need re-resolution and possible rule updates"

## Why This Matters

Without this shift, Whetstone will remain interesting but cumbersome.
With it, Whetstone becomes practical for real repos, repeat usage, and eventual open-source adoption.

## Success Criteria

Whetstone v2 succeeds if:

- large monorepos get useful feedback quickly
- repeated runs are mostly incremental
- unfinished long-running work can resume safely
- users can act on partial results immediately
- status clearly distinguishes changed deps, stale docs, cached results, and extraction-ready candidates

## Non-Goals

Whetstone v2 is not trying to:

- become a general PR review bot
- fully automate rule approval
- auto-fix arbitrary code issues
- prioritize specific frameworks by hand over a generic ranking model

## Summary

Whetstone v2 should move from a promising blocking workflow to a fast, cached, incremental pipeline for deriving dependency-backed coding rules in real projects.
