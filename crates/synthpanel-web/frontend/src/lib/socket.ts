/**
 * Minimal WebSocket client with automatic reconnection.
 *
 * The panel pushes live metrics over `/ws/dashboard`; the dashboard route
 * subscribes to it and falls back to polling when the socket is unavailable.
 */

export interface SocketOptions {
  /** Called for every parsed message. */
  onMessage: (payload: unknown) => void;
  /** Called when the connection drops. */
  onClose?: () => void;
  /** Reconnect delay in milliseconds. */
  retryDelay?: number;
}

export function connectSocket(path: string, options: SocketOptions): () => void {
  const retryDelay = options.retryDelay ?? 3000;
  let socket: WebSocket | null = null;
  let closed = false;
  let timer: ReturnType<typeof setTimeout> | undefined;

  const open = () => {
    if (closed) return;
    const scheme = window.location.protocol === 'https:' ? 'wss' : 'ws';
    socket = new WebSocket(`${scheme}://${window.location.host}${path}`);

    socket.onmessage = (event) => {
      try {
        options.onMessage(JSON.parse(event.data as string));
      } catch {
        // Ignore frames that are not JSON.
      }
    };
    socket.onclose = () => {
      options.onClose?.();
      if (!closed) {
        timer = setTimeout(open, retryDelay);
      }
    };
    socket.onerror = () => {
      socket?.close();
    };
  };

  open();

  return () => {
    closed = true;
    if (timer) clearTimeout(timer);
    socket?.close();
  };
}
