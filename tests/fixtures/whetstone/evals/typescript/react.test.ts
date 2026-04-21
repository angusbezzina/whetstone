import { describe, it, expect } from 'vitest';
import { findSourceFiles, readLines } from './setup';

describe('react.use-client-directive', () => {

  it('signal 0: File uses useState/useEffect but lacks \'use client\' directive', () => {
    const files = findSourceFiles();
    const violations: string[] = [];
    for (const file of files) {
      const lines = readLines(file);
      lines.forEach((line, idx) => {
        // TODO: add `match:` regex to rule react.use-client-directive signal has-hooks-no-directive to enable this check.
      });
    }
    expect(violations).toEqual([]);  // react.use-client-directive
  });


  it('signal 1: File accesses window/document without \'use client\'', () => {
    const files = findSourceFiles();
    const violations: string[] = [];
    for (const file of files) {
      const lines = readLines(file);
      lines.forEach((line, idx) => {
        // TODO: add `match:` regex to rule react.use-client-directive signal uses-browser-api to enable this check.
      });
    }
    // NOTE: ast signal regex fallback — upgrade to tree-sitter when available.
    expect(violations).toEqual([]);  // react.use-client-directive
  });

});
