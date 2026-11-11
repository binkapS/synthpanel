/**
 * i18n scaffolding.
 *
 * English is the reference locale. Translations live in the `messages` map;
 * adding a locale is a matter of adding a dictionary and a `<option>` in the
 * settings page. Untranslated keys fall back to English.
 */
import { derived, writable } from 'svelte/store';

export const locales = ['en'] as const;
export type Locale = (typeof locales)[number];

const stored = localStorage.getItem('synthpanel-locale');
export const locale = writable<Locale>(
  locales.includes(stored as Locale) ? (stored as Locale) : 'en',
);

locale.subscribe((value) => {
  localStorage.setItem('synthpanel-locale', value);
  document.documentElement.lang = value;
});

const en: Record<string, string> = {
  'app.name': 'SynthPanel',
  'app.tagline': 'Live server overview',
  'nav.dashboard': 'Dashboard',
  'nav.sites': 'Sites',
  'nav.databases': 'Databases',
  'nav.php': 'PHP',
  'nav.security': 'Security',
  'nav.backups': 'Backups',
  'nav.cron': 'Cron',
  'nav.logs': 'Logs',
  'nav.files': 'Files',
  'nav.deployments': 'Deployments',
  'nav.alerts': 'Alerts',
  'nav.terminal': 'Terminal',
  'nav.users': 'Users',
  'nav.settings': 'Settings',
  'nav.main': 'Main navigation',
  'nav.skip': 'Skip to content',
  'login.title': 'Sign in to SynthPanel',
  'login.subtitle': 'Manage your server from a single place.',
  'login.email': 'Email',
  'login.password': 'Password',
  'login.code': 'Authentication code',
  'login.codeHint': 'Enter the 6-digit code from your authenticator app.',
  'login.submit': 'Sign in',
  'login.submitting': 'Signing in…',
  'common.save': 'Save changes',
  'common.saving': 'Saving…',
  'settings.update': 'Panel update',
  'settings.updateSubtitle': 'signed releases with automatic rollback',
  'settings.channel': 'Channel',
  'settings.check': 'Check for updates',
  'settings.apply': 'Install update',
  'settings.upToDate': 'The panel is up to date.',
  'settings.updateAvailable': 'Version {version} is available.',
};

const messages: Record<Locale, Record<string, string>> = { en };

function translate(locale: Locale, key: string, vars?: Record<string, string | number>): string {
  let text = messages[locale]?.[key] ?? en[key] ?? key;
  if (vars) {
    for (const [name, value] of Object.entries(vars)) {
      text = text.replaceAll(`{${name}}`, String(value));
    }
  }
  return text;
}

/** Reactive translator: use `{$t('nav.dashboard')}` in templates. */
export const t = derived(locale, ($locale) => {
  return (key: string, vars?: Record<string, string | number>) => translate($locale, key, vars);
});

export function setLocale(value: string): void {
  if (locales.includes(value as Locale)) {
    locale.set(value as Locale);
  }
}
