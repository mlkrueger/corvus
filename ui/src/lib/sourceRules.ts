// Pure parse/format of source rule lines: `AA, LAI: To Do, In Progress` (`*` = any project). Used by plan 0008.

export interface SourceRule {
  /** Project keys; `['*']` means any project. */
  projects: string[];
  statuses: string[];
}

export interface ParseResult {
  rules: SourceRule[];
  /** One entry per rejected line (1-based line number). */
  errors: { line: number; text: string; message: string }[];
}

function splitList(s: string): string[] {
  return s
    .split(',')
    .map((x) => x.trim())
    .filter((x) => x.length > 0);
}

/** Parses one rule per line; blank lines are skipped. Statuses may contain spaces ("In Progress"). */
export function parseRules(text: string): ParseResult {
  const rules: SourceRule[] = [];
  const errors: ParseResult['errors'] = [];
  text.split(/\r?\n/).forEach((raw, i) => {
    const line = raw.trim();
    if (line === '') return;
    const colon = line.indexOf(':');
    if (colon < 0) {
      errors.push({ line: i + 1, text: raw, message: 'expected "PROJECTS: STATUSES"' });
      return;
    }
    const projects = splitList(line.slice(0, colon));
    const statuses = splitList(line.slice(colon + 1));
    if (projects.length === 0) {
      errors.push({ line: i + 1, text: raw, message: 'no projects before ":" (use * for any)' });
      return;
    }
    if (statuses.length === 0) {
      errors.push({ line: i + 1, text: raw, message: 'no statuses after ":"' });
      return;
    }
    rules.push({ projects: projects.includes('*') ? ['*'] : projects, statuses });
  });
  return { rules, errors };
}

export function formatRule(rule: SourceRule): string {
  return `${rule.projects.join(', ')}: ${rule.statuses.join(', ')}`;
}

export function formatRules(rules: SourceRule[]): string {
  return rules.map(formatRule).join('\n');
}
