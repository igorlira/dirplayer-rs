import { useSyncExternalStore } from 'react';

// 'system' follows the OS; the other two are explicit overrides that win over
// it. See styles/theme.css for the matching cascade.
export type ThemePreference = 'system' | 'light' | 'dark';
export type ResolvedTheme = 'light' | 'dark';

const THEME_KEY = 'theme';
const DARK_QUERY = '(prefers-color-scheme: dark)';

export function loadThemePreference(): ThemePreference {
  try {
    const stored = window.localStorage.getItem(THEME_KEY);
    if (stored === 'light' || stored === 'dark' || stored === 'system') {
      return stored;
    }
  } catch {
    /* private mode / storage disabled */
  }
  return 'system';
}

function detectSystemTheme(): ResolvedTheme {
  return window.matchMedia?.(DARK_QUERY).matches ? 'dark' : 'light';
}

export function resolveTheme(preference: ThemePreference): ResolvedTheme {
  return preference === 'system' ? detectSystemTheme() : preference;
}

// The attribute drives the CSS; 'system' removes it so the media query decides.
export function applyThemePreference(preference: ThemePreference) {
  const root = document.documentElement;
  if (preference === 'system') {
    root.removeAttribute('data-theme');
  } else {
    root.setAttribute('data-theme', preference);
  }
}

// --- Shared store -----------------------------------------------------------
// One source of truth for every consumer. Component-local state does not work
// here: most of the theme is CSS variables that switch off the <html>
// attribute, but a few consumers (the script viewer's syntax palette) need the
// resolved value in JS. With per-component state those consumers kept whatever
// value they mounted with, so toggling the theme restyled the chrome and left
// their text on the old palette.

let preference: ThemePreference = 'system';
let systemTheme: ResolvedTheme = 'light';
let snapshot = 'system|light';
const listeners = new Set<() => void>();

function refresh() {
  const next = `${preference}|${systemTheme}`;
  if (next === snapshot) return;
  // useSyncExternalStore compares snapshots by identity, so this must be a
  // primitive that only changes when something actually changed.
  snapshot = next;
  listeners.forEach(listener => listener());
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot() {
  return snapshot;
}

export function setThemePreference(next: ThemePreference) {
  preference = next;
  applyThemePreference(next);
  try {
    window.localStorage.setItem(THEME_KEY, next);
  } catch {
    /* preference just won't persist */
  }
  refresh();
}

// Called before React mounts so the first paint is already themed.
export function initTheme() {
  preference = loadThemePreference();
  systemTheme = detectSystemTheme();
  snapshot = `${preference}|${systemTheme}`;
  applyThemePreference(preference);

  // Track the OS flipping (e.g. macOS auto appearance). Only matters while on
  // 'system', but keeping it current means no stale value if the user switches
  // back to it later.
  const query = window.matchMedia?.(DARK_QUERY);
  query?.addEventListener('change', () => {
    systemTheme = query.matches ? 'dark' : 'light';
    refresh();
  });
}

export function useTheme() {
  const current = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  const [storedPreference, resolvedSystem] = current.split('|') as [ThemePreference, ResolvedTheme];
  return {
    preference: storedPreference,
    resolved: storedPreference === 'system' ? resolvedSystem : storedPreference,
    setPreference: setThemePreference,
  };
}
