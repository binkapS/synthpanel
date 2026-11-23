# Migration Plan: Svelte → Slint (WebAssembly)

## Status

Phases 0–6 implemented in commits `18f5868`, `9bcfb62`, `7f7c343`, `5784ee2`, `86cdc8c`, `e09351a`, `93ad92d`.

- Slint WASM UI lives in `crates/synthpanel-web/ui/` and is embedded by `synthpanel-web`.
- CI builds the WASM UI; the old Node frontend CI job was removed.
- The legacy Svelte frontend has been removed from the repository; Playwright E2E selectors still need migration to the Slint/canvas UI.
- Remaining work: browser/runtime acceptance pass, canvas-friendly E2E strategy, terminal parity improvements, and optional WASM size optimization (`wasm-opt`).

## 1. What changes — and what doesn't

### Stays the same
- `synthpanel-core` — no changes
- `synthpanel-ipc` — protocol unchanged
- `synthpanel-web/src/api/*` — REST endpoints unchanged
- `synthpanel-web/src/ws.rs` — WebSocket handlers unchanged
- TLS, auth, session, CSRF, ratelimit middleware — unchanged
- Installation, packaging, systemd units — unchanged

### Changes
| Current | New |
|---|---|
| `crates/synthpanel-web/frontend/` — Svelte 5 + TypeScript + Vite | `crates/synthpanel-web/ui/` — Slint `.slint` sources |
| `npm run build` → Vite emits JS/CSS chunks to `dist/` | `cargo build` → Slint emits `.wasm` + generated Rust glue |
| `include_dir!("frontend/dist")` embeds static assets | Same `include_dir!`, pointing to a WASM output dir |
| Browser runs an ES module bootstrapping the Svelte app | Browser loads a tiny `slint-loader.js` that boots the WASM module |
| SPA router driven by `lib/router.ts` (History API) | Navigation driven by Slint property binding (`current-page` enum) |
| `$state` / `$derived` / stores in TS | Slint `property<T>` + Rust-side models |

### Why this makes sense
The panel ships as a single HTTPS process with zero external deployment targets. Moving to Slint+WASM keeps that property — the WASM blob is embedded in the binary the same way the JS bundle is today — while unifying the frontend into the Rust toolchain, removing Node/npm from the build pipeline entirely, and giving access to native Rust libraries (e.g., a real terminal emulator instead of the ~68 kB xterm.js chunk).

---

## 2. Architecture

```
Browser (any modern browser)
  │
  │  HTTP/S (port 8443)
  ▼
synthpanel-web (axum, Rust)
  ├── /api/v1/*              → existing REST handlers (unchanged)
  ├── /ws/*                  → existing WebSocket handlers (unchanged)
  ├── /.well-known/*         → ACME challenge (unchanged)
  └── /*                     → fallback → serve ui/embedded/index.html
         │
         │  <script src="/slint-loader.js"></script>
         │  Embedded WASM module (~1.5 MB) + tiny bootstrap (~2 kB)
         ▼
      slint.wasm
         ├── app.slint         root — session gate + shell
         ├── widgets/          shared components
         │     card.slint
         │     stat_card.slint
         │     status_badge.slint
         │     empty_state.slint
         │     sparkline.slint
         │     service_list.slint
         │     audit_feed.slint
         │     app_shell.slint
         └── pages/            route components
               login.slint
               dashboard.slint
               sites.slint
               site_detail.slint
               databases.slint
               php.slint
               security.slint
               users.slint
               backups.slint
               cron.slint
               logs.slint
               files.slint
               deployments.slint
               alerts.slint
               terminal.slint
               settings.slint
```

**Slint loader.js** (~2 kB): bootstraps the WASM module, connects `window.synthpanel_api` (the JS bridge populated by the Rust side) to Slint callback properties, mounts the root component. No framework runtime beyond what Slint produces.

---

## 3. Build Pipeline

### Current
```
frontend/package.json
  → npm ci
  → vite build       (ESM chunks + CSS extracted)
  → include_dir!(dist)   baked into synthpanel-web at cargo compile time
```

### New
```
ui/build.rs           (custom, invokes slintbuild + wasm-pack)
Cargo.toml            ([dependencies] + [build-dependencies])
  → cargo build -p synthpanel-web    (runs build.rs, emits .wasm)
  → include_dir!(ui/dist)            baked in at compile time
```

**Two options for the WASM compilation step:**

| Option | Tooling | Pros | Cons |
|---|---|---|---|
| **A — `wasm-pack`** | `wasm-pack build --target web` | Mature, good source maps, familiar ergonomics | Requires Nightly Rust for some features; extra CLI tool |
| **B — `slint-build` + manual `wasm-bindgen`** | `slint-build --emit rsrc` + standalone wasm-bindgen | Explicit control, no extra tool dependency beyond toolchain | More boilerplate in `build.rs` |

**Recommended: Option A.** `wasm-pack` produces a clean ES-module-compatible WASM with bindings. The output can be dropped straight into the embedding directory.

---

## 4. Phase Breakdown

### Phase 0 — Scaffold & Hello World (Week 1)

**Goal:** a compilable binary serving a Slint-rendered page over HTTPS, proving the end-to-end pipeline.

1. Add `slint = { version = "1.9", default-features = false }` and `slint-backend-browser` to `synthpanel-web/Cargo.toml`.
2. Add `wasm-pack` as a dev-dependency (or install globally in CI).
3. Create `crates/synthpanel-web/ui/` with:
   - `ui/app.slint` — a single window with "Hello SynthPanel" text
   - `ui/pkg/` — gitignored, produced by wasm-pack
   - `ui/build.rs` — script that runs `wasm-pack build --out-dir pkg --target web` before the Rust crate build
4. Replace `static_files.rs`'s `include_dir!` path to point at `ui/dist` (or the wasm-pack output).
5. Serve a minimal `index.html`:
   ```html
   <!doctype html>
   <html lang="en"><body class="h-full">
     <div id="app" class="h-full"></div>
     <script type="module">
       import init from './synthpanel_wasm.js';
       init().then(app => {
         const instance = app.run();
         document.getElementById('app').replaceWith(instance.window().dom_element());
       });
     </script>
   </body></html>
   ```
6. Verify: running `cargo build --release -p synthpanel-web` produces a working binary; accessing `https://localhost:8443/` renders the Slint window.

**Exit criterion:** `cargo build --release -p synthpanel-web` succeeds; the binary serves a Slint UI over HTTPS; no npm/node involved anywhere.

---

### Phase 1 — Shell, Session Gate & Theming (Week 1–2)

**Goal:** the skeleton layout the user sees after authentication, matching the current AppShell structure.

1. Port `AppShell.svelte` → `widgets/app_shell.slint`:
   - Sidebar with 15 nav items using Slint `ListView` or a `Row` of `Button`s
   - Active-state highlighting bound to a `current-page` property
   - Top bar: logo, theme toggle (light/dark), user name+role badge, logout button
   - Accessible skip-link (`tab-index: -1`)
2. Port the icon set — 15 Lucide icons used in the nav. Extract the SVGs from `@lucide/svelte` package and inline them as Slint image resources (Slint supports SVG natively).
3. Implement `app.slint` root:
   - Enum `Page` with all 15 routes
   - Property `session: User?` (null = not logged in)
   - Conditional: show `<LoginPage />` when session is null, else `<AppShell currentPage={...}>`
4. Wire authentication:
   - Expose a Slint callback `on-login(email, password, code) -> Result<User, Error>`
   - In Rust, implement this callback by calling `/api/v1/auth/login` via `reqwest` (cookie jar persists the session)
   - Store cookies in a `reqwest::CookieStore` kept in the app state
5. Theme persistence: write to a config file (`~/.config/synthpanel/theme`) instead of `localStorage`; read it during app init and set Slint's `default-color-scheme`.
6. i18n scaffold: create `locales/en.slint` with all the same message keys from `i18n.ts`. Use Slint's built-in localization via `import "en"` at the top of each `.slint` file.

**Exit criterion:** Login form authenticates against the live API, cookie session persists across page reloads; clicking nav items switches the content area; dark/light toggle works and survives restart.

---

### Phase 2 — Dashboard (Week 2)

**Goal:** port the most data-rich page to validate the async/WASM communication pattern.

1. Port components:
   - `widgets/card.slint` — bordered container with optional title/subtitle
   - `widgets/stat_card.slint` — icon + value + hint (matches current StatCard)
   - `widgets/sparkline.slint` — uses Slint's canvas to draw an SVG-like polyline from a `property <array<float>> values`
   - `widgets/service_list.slint` — iterates services with status badges
   - `widgets/audit_feed.slint` — table of recent audit entries
2. Implement `pages/dashboard.slint`:
   - 4 stat cards (CPU, memory, disk, network)
   - CPU + memory sparklines (last 24 h buffer)
   - Service list + audit feed side-by-side
   - Quick-action buttons navigating to sites/backups/terminal
3. Handle real-time metrics:
   - On mount: fetch snapshot + range from `/api/v1/metrics/snapshot` and `/api/v1/metrics/range?range=24h`
   - Maintain a sliding window (`Vec<(u64, f32)>`) in a Rust struct, exposed to Slint via a `#[model]` or a refreshed `property <array<float>>`
   - Open a persistent WebSocket to `/ws/dashboard` via the same `synthpanel-web/src/ws.rs` handler (no changes needed there)
   - On each WS message, update the metrics buffer → notify Slint property → sparkline redraws
   - Fallback to polling every 5 s when the socket is closed (port the existing `connectSocket` logic into Rust)

**Exit criterion:** Dashboard renders identically to the current SPA, auto-refreshes via WebSocket, shows live CPU/memory/network stats.

---

### Phase 3 — CRUD Pages (Week 3–4)

**Goal:** port the data-heavy pages with forms, tables, and mutations.

Port in order of complexity (simplest first, so the developer establishes a reusable pattern):

| Priority | Page | Key Patterns |
|---|---|---|
| 1 | Sites list + Site detail | Table view, tabbed sub-view |
| 2 | Settings | Multiple sections, toggle switches |
| 3 | Users | List + create/edit modal |
| 4 | Security | Firewall rules table, fail2ban, SSH keys |
| 5 | Databases | Create DB/user, privilege selector |
| 6 | Backups | Schedule editor, restore actions |
| 7 | Cron | Job list, schedule expression |
| 8 | Deployments | Git repo input, run history |
| 9 | Alerts | Channel/rule CRUD |
| 10 | PHP | Version selector |

**Reusable patterns established early:**
- `FormSheet.slint` — reusable modal/drawer with title, field slots, save/cancel
- `ConfirmDialog.slint` — destructive action confirmation
- `StatusBadge.slint` — colored dot + label (green/yellow/red)
- `DataTable.slint` — generic scrollable table with column definitions as a Slint struct

Each page follows the template:
```slint
// In the page definition
callback on-save(params) -> Result<Void, Error>;
callback on-delete(id) -> Result<Void, Error>;

// Called from Rust via #[component] callbacks
component LoginPage { ... }
component DashboardPage { ... }
// etc.
```

Rust side provides:
```rust
impl MyPageCallbacks for AppState {
    fn on_save(&self, params: serde_json::Value) -> Result<(), ApiError> {
        // call core via ipc
    }
}
```

**Exit criterion:** All 14 data pages render and perform mutations against the live API.

---

### Phase 4 — Terminal (Week 4)

**Goal:** replace xterm.js (~68 kB gzip) with a native terminal widget.

The SPA terminal opens a WebSocket to `/ws/terminal/:id` and pipes bytes back and forth. The WebSocket protocol (defined in `ws.rs:terminal_ws`) stays identical — no server changes needed.

Implementation approaches, ranked by effort:

| Approach | Effort | Notes |
|---|---|---|
| **A: Slint Canvas + ANSI renderer** | Medium | Implement a virtual terminal buffer in Rust, render to Slint's Canvas. Reuses the exact same WS frames. |
| **B: `tui-rs` rendered to bitmap** | High | Full TUI stack; heavier, harder to integrate with Slint's event loop. |
| **C: Reuse xterm.js via wasm-bindgen** | Low | Keep xterm as a separate WASM bundle; complex coupling. |

**Chosen: Approach A.** Slint's Canvas element lets us draw individual cells as rectangles with text. A lightweight VT100-compatible buffer (scrollback, cursor, attributes) maintains the terminal state in Rust and draws on each frame. This replaces xterm.js entirely while preserving the same WebSocket protocol.

Features to implement:
- VT100 escape sequence parsing (basic: color, bold, cursor movement, scroll)
- Scrollback buffer (fixed size, e.g. 1000 lines)
- Keyboard input forwarded as-is to the WebSocket
- Resize events forwarded as `{type: "resize", rows, cols}` frames
- Dark theme background matching current `#0b1220`

**Exit criterion:** Functional terminal with scrollback, paste, resize, and correct color rendering.

---

### Phase 5 — Polish & Parity (Week 5)

1. **Styling audit**: walk through `app.css` and verify each Tailwind utility used has a Slint equivalent. Slint handles dark-mode colors via `dark:`-style overrides or dual-theme definitions — audit for any missed transitions.
2. **Accessibility**: ensure all interactive elements declare `accessible-name`, tab navigation order is correct, focus indicators are visible.
3. **Performance**: target the same <150 kB payload budget. Profile the WASM module size; strip debug symbols in release builds. The current 57 kB gzip benchmark is a floor to beat.
4. **Service Worker removal**: the current PWA support (`sw.js`, manifest, meta tags) only benefits the Svelte app. Decide whether to drop PWA support or add a minimal SW that caches the WASM blob.
5. **E2E validation**: compare behavior against the Svelte SPA under the same test scenarios (login, site creation, backup, log tail, terminal session).

---

### Phase 6 — Deprecate & Ship (Week 6)

1. Update `Cargo.toml` workspace members to include the new WASM build step.
2. Update `.github/workflows/ci.yml`:
   - Remove the `frontend` job (Node/npm install + Vite build)
   - Add a WASM build step to the main quality job (ensure `wasm-pack` is available in the Rust toolchain image)
3. Update `packaging/build-deb.sh`: no change needed — the WASM output is still embedded in `synthpanel-web` via `include_dir!`.
4. Update `PLAN.md`: change the frontend technology row from "Svelte 5 + TypeScript" to "Slint (compiled to WASM)".
5. Mark `crates/synthpanel-web/frontend/` as deprecated in a comment block at the top of each retained file, and remove it once acceptance testing passes.

---

## 5. Component Mapping Reference

| Svelte | Slint | File |
|---|---|---|
| `App.svelte` | `app.slint` | Root — session gate + shell wrapper |
| `AppShell.svelte` | `widgets/app_shell.slint` | Sidebar + header + content area |
| `Card.svelte` | `widgets/card.slint` | Bordered container |
| `StatCard.svelte` | `widgets/stat_card.slint` | Icon + value + hint |
| `StatusBadge.svelte` | `widgets/status_badge.slint` | Dot + label |
| `EmptyState.svelte` | `widgets/empty_state.slint` | Title + description + CTA |
| `Sparkline.svelte` | `widgets/sparkline.slint` | Canvas line chart |
| `ServiceList.svelte` | `widgets/service_list.slint` | Service status table |
| `AuditFeed.svelte` | `widgets/audit_feed.slint` | Audit timeline |
| `routes/Dashboard.svelte` | `pages/dashboard.slint` | Aggregates dashboard widgets |
| `routes/Login.svelte` | `pages/login.slint` | Email/password + 2FA |
| `routes/Sites.svelte` | `pages/sites.slint` | Site list table |
| `routes/SiteDetail.svelte` | `pages/site_detail.slint` | Tabbed detail view |
| `routes/SiteDetailRoute.svelte` | *(merged)* | Router redirect → direct Nav action |
| `routes/Databases.svelte` | `pages/databases.slint` | DB list + create form |
| `routes/Php.svelte` | `pages/php.slint` | Version manager |
| `routes/Security.svelte` | `pages/security.slint` | Firewall/fail2ban/SSH keys |
| `routes/Users.svelte` | `pages/users.slint` | User CRUD |
| `routes/Backups.svelte` | `pages/backups.slint` | Backup list + schedules |
| `routes/Cron.svelte` | `pages/cron.slint` | Job editor |
| `routes/Logs.svelte` | `pages/logs.slint` | Log viewer + filters |
| `routes/Files.svelte` | `pages/files.slint` | File manager |
| `routes/Deployments.svelte` | `pages/deployments.slint` | Deployment list |
| `routes/Alerts.svelte` | `pages/alerts.slint` | Alert channels/rules |
| `routes/Terminal.svelte` | `pages/terminal.slint` | VT terminal widget |
| `routes/Settings.svelte` | `pages/settings.slint` | Panel settings |

---

## 6. Key Technical Decisions

### 6.1 State Management
Svelte runes (`$state`, `$derived`) map to:
- `property<T>` inside `.slint` components for per-component local state
- Rust-side model objects (e.g. `DashboardMetrics`) holding live data, exposed to Slint via `#[model]` or by updating a bound property from Rust callbacks
- Session state lives in a Rust struct (`AppState`) and is injected into the Slint app via the root component's constructor

### 6.2 Networking Bridge
Instead of in-browser `fetch()` and `new WebSocket()`:
- REST calls: `reqwest::Client` with a `CookieJar` managed by the Rust app state
- Callbacks exposed to Slint as `callback` properties that invoke Rust functions
- WebSocket connections (dashboard stream, log tail, terminal) managed by dedicated Tokio tasks in Rust, pushing updates into Slint properties via the generated bindings

The existing `ws.rs` handlers need **zero changes** — they speak the same wire protocol.

### 6.3 Routing
Slint has no URL concept. Navigation is driven by a single `current_page` enum property on the root component. Deep-linking on initial load is supported via a command-line argument or a query string parsed by a tiny JS shim. History-backed back-button is not implemented — acceptable for a control panel where the sidebar is the primary navigation.

### 6.4 Styling
Tailwind utilities map to Slint style declarations:

| Tailwind | Slint equivalent |
|---|---|
| `flex` / `grid` | `Row { spacing: ... }` / `Grid { rows: N; columns: M; }` |
| `p-4`, `m-2`, `gap-3` | `padding: 1rem; spacing: 0.75rem;` |
| `rounded-lg` | `border-radius: 0.5rem;` |
| `bg-white`, `text-slate-700` | `background: white; color: #334155;` |
| `dark:bg-slate-900` | `dark-background: #0f172a;` (Slint's built-in dark mode) |
| `hover:bg-slate-50` | `MouseArea { hovered: true → background: ... }` |

The `@custom-variant dark (&:where(.dark, .dark *))` Tailwind directive maps directly to Slint's `dark:` property namespace — no custom plugin needed.

### 6.5 Icons
Extract the 15 Lucide SVGs used in the nav:
```bash
# From node_modules/@lucide/svelte/dist/icons/*.svg
for icon in LayoutDashboard Globe Database Server ShieldCheck Archive Clock \
            ScrollText FolderTree GitBranch BellRing TerminalSquare Users Settings Activity; do
  cp node_modules/@lucide/svelte/dist/icons/${icon}.svg ui/assets/$icon.svg
done
```
Reference in `.slint` files as `image: url("icons/LayoutDashboard.svg");` — Slint supports inline SVG natively.

---

## 7. Risk Register

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| WASM module >150 kB (bundle budget) | Medium | Medium | Profile early; Slint's WASM output is typically ~400–800 kB uncompressed — well above the current 57 kB gzip. May need to negotiate a higher budget given the trade-off (single Rust crate vs. separate Node toolchain). |
| Slint Canvas performance for terminal | Medium | High | Benchmark the VT renderer before Phase 4; if too slow, fall back to a lightweight external dependency |
| Cookie-based auth in WASM context | Low | Medium | `reqwest` cookie jar is Rust-side; the browser never touches session cookies directly — this is actually simpler than the Svelte version |
| Slint version lock-in | Low | Low | Pin to a specific Slint minor version; track upstream releases quarterly |
| Lost PWA/offline support | Medium | Low | Service worker was minor; document the loss and reassess post-migration |
| Debugging WASM in production panels | Low | High | Ensure source maps are included in development builds; add a `--debug-ui` flag for verbose WASM logging |

---

## 8. Acceptance Criteria

The migration is complete when:

1. `cargo build --release -p synthpanel-web` compiles without any Node/npm dependencies
2. All 14 functional pages render correctly with visual parity to the Svelte SPA
3. Every API endpoint exercised by the SPA is accessible from the Slint UI
4. Real-time features work: dashboard auto-refresh via WebSocket, log tail, terminal session
5. Dark and light themes are visually consistent with the current palette
6. The `.deb` packages contain the Slint-built binary with no leftover frontend artifacts
7. E2E tests pass against the Slint UI (or a Playwright instance loading the WASM page)
8. `PLAN.md` §5.3 and §9.3 are updated to reflect the new frontend technology
9. `crates/synthpanel-web/frontend/` is removed (or marked deprecated and removed after a grace period)
