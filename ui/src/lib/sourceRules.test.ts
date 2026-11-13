import { describe, expect, it } from 'vitest';
import { formatRule, formatRules, parseRules } from './sourceRules';

describe('sourceRules', () => {
  it('parses the documented example', () => {
    const { rules, errors } = parseRules('AA, LAI: To Do, In Progress');
    expect(errors).toEqual([]);
    expect(rules).toEqual([{ projects: ['AA', 'LAI'], statuses: ['To Do', 'In Progress'] }]);
  });

  it('treats * as any project', () => {
    expect(parseRules('*: Done').rules).toEqual([{ projects: ['*'], statuses: ['Done'] }]);
    expect(parseRules('AA, *: Done').rules[0]!.projects).toEqual(['*']);
  });

  it('round-trips parse -> format -> parse', () => {
    const text = 'AA, LAI: To Do, In Progress\n*: Ready\nZZ: Blocked';
    const first = parseRules(text);
    expect(formatRules(first.rules)).toBe(text);
    expect(parseRules(formatRules(first.rules)).rules).toEqual(first.rules);
  });

  it('normalises whitespace on format', () => {
    const { rules } = parseRules('  AA ,LAI :  To Do ,In Progress  ');
    expect(formatRule(rules[0]!)).toBe('AA, LAI: To Do, In Progress');
  });

  it('skips blank lines, handles CRLF, and reports bad lines with their number', () => {
    const { rules, errors } = parseRules('AA: Open\r\n\r\nnonsense\n: Open\nAA:\n');
    expect(rules).toEqual([{ projects: ['AA'], statuses: ['Open'] }]);
    expect(errors.map((e) => e.line)).toEqual([3, 4, 5]);
  });

  it('empty input is no rules and no errors', () => {
    expect(parseRules('')).toEqual({ rules: [], errors: [] });
    expect(formatRules([])).toBe('');
  });
});
