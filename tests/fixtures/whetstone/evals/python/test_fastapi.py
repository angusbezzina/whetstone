"""Whetstone evals for dependency: fastapi."""


# Rule: fastapi.async-routes — Route handlers MUST use async def.

def test_fastapi_async_routes_signal_0(python_source_files):
    """Signal: Function decorated with route decorator uses def instead of async def (ast)"""
    violations = []
    for filepath in python_source_files:
        with open(filepath, encoding="utf-8") as f:
            for lineno, line in enumerate(f, 1):
                pass  # TODO: add `match:` regex to rule fastapi.async-routes signal is-sync-function to enable this check.
    # NOTE: ast signal regex fallback — replace with tree-sitter when available.
    assert not violations, f"{len(violations)} violation(s) for fastapi.async-routes: {violations[:5]}"

