/// <reference types="svelte" />
/// <reference types="vite/client" />

// The xterm stylesheet is imported dynamically so the terminal chunk is only
// fetched when the terminal page is opened.
declare module '@xterm/xterm/css/xterm.css';
