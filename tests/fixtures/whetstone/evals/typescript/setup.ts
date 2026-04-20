import { glob } from 'glob';
import { readFileSync } from 'fs';

export function findSourceFiles(patterns: string[] = ['src/**/*.ts', 'src/**/*.tsx']): string[] {
  return patterns.flatMap(p => glob.sync(p, { ignore: ['**/node_modules/**', '**/whetstone/**'] }));
}

export function readLines(filepath: string): string[] {
  return readFileSync(filepath, 'utf-8').split('\n');
}

export interface Violation {
  file: string;
  line: number;
  text: string;
}

export function violation(file: string, line: number, text: string): Violation {
  return { file, line, text };
}
