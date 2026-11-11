import { writable } from 'svelte/store';
import { api } from './api';

export interface User {
  id: string;
  email: string;
  name: string;
  role: string;
  status: string;
  totp_enabled: boolean;
  quota_sites: number;
  quota_dbs: number;
  last_login_at: string | null;
  created_at: string;
}

export interface SessionState {
  user: User;
}

export const session = writable<SessionState | null>(null);
export const loading = writable(true);

export type Theme = 'light' | 'dark';

const storedTheme = (localStorage.getItem('synthpanel-theme') as Theme | null) ?? 'light';
export const theme = writable<Theme>(storedTheme);

theme.subscribe((value) => {
  localStorage.setItem('synthpanel-theme', value);
  document.documentElement.classList.toggle('dark', value === 'dark');
});

export function toggleTheme(): void {
  theme.update((value) => (value === 'dark' ? 'light' : 'dark'));
}

export async function fetchMe(): Promise<User> {
  return api<User>('/api/v1/auth/me');
}

export async function loadSession(): Promise<User | null> {
  try {
    const user = await fetchMe();
    session.set({ user });
    return user;
  } catch {
    session.set(null);
    return null;
  }
}

export async function logout(): Promise<void> {
  try {
    await api('/api/v1/auth/logout', { method: 'POST' });
  } finally {
    session.set(null);
  }
}
