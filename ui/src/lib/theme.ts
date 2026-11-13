// Light/dark theme via CSS custom properties. `prefers-color-scheme` is the default; the toggle overrides it.
export type ThemeChoice = 'light' | 'dark';

const KEY = 'corvus-theme';

export function storedTheme(): ThemeChoice | null {
  try {
    const v = localStorage.getItem(KEY);
    return v === 'light' || v === 'dark' ? v : null;
  } catch {
    return null;
  }
}

export function systemTheme(): ThemeChoice {
  return typeof matchMedia === 'function' && matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

export function effectiveTheme(): ThemeChoice {
  return storedTheme() ?? systemTheme();
}

export function applyTheme(choice: ThemeChoice | null): void {
  if (choice) document.documentElement.setAttribute('data-theme', choice);
  else document.documentElement.removeAttribute('data-theme');
}

export function saveTheme(choice: ThemeChoice): void {
  try {
    localStorage.setItem(KEY, choice);
  } catch {
    // Storage unavailable: the choice lasts for this session only.
  }
  applyTheme(choice);
}

export function initTheme(): void {
  applyTheme(storedTheme());
}
