import { readable } from 'svelte/store';

/** Current browser path. Hash-free routing backed by the History API. */
export const currentPath = readable(window.location.pathname, (set) => {
  const update = () => set(window.location.pathname);
  window.addEventListener('popstate', update);
  return () => window.removeEventListener('popstate', update);
});

export function navigate(to: string): void {
  if (to === window.location.pathname) return;
  window.history.pushState({}, '', to);
  window.dispatchEvent(new PopStateEvent('popstate'));
}
