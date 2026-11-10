export interface ApiErrorBody {
  error: { code: string; message: string };
}

export class ApiError extends Error {
  status: number;
  code: string;

  constructor(status: number, code: string, message: string) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
    this.code = code;
  }
}

export function readCookie(name: string): string | null {
  const prefix = `${name}=`;
  const entry = document.cookie
    .split(';')
    .map((part) => part.trim())
    .find((part) => part.startsWith(prefix));
  return entry ? decodeURIComponent(entry.slice(prefix.length)) : null;
}

export async function api<T>(path: string, init: RequestInit = {}): Promise<T> {
  const headers = new Headers(init.headers);
  if (init.body !== undefined && !headers.has('content-type')) {
    headers.set('content-type', 'application/json');
  }
  const method = (init.method ?? 'GET').toUpperCase();
  if (method !== 'GET' && method !== 'HEAD') {
    const csrf = readCookie('synthpanel_csrf');
    if (csrf) headers.set('x-csrf-token', csrf);
  }

  const response = await fetch(path, {
    ...init,
    headers,
    credentials: 'same-origin',
  });

  const text = await response.text();
  let payload: unknown = null;
  if (text.length > 0) {
    try {
      payload = JSON.parse(text);
    } catch {
      payload = text;
    }
  }

  if (!response.ok) {
    const body = payload as ApiErrorBody | null;
    const code = body?.error?.code ?? 'error';
    const message = body?.error?.message ?? response.statusText ?? 'request failed';
    throw new ApiError(response.status, code, message);
  }

  return payload as T;
}
