# Migration Plan: Svelte → Slint UI

## 1. Overview

### Current State
- **18 route `.svelte` files** (~2,900 lines total)
- **8 shared components** (~400 lines total)
- Svelte 5 runes (`$state`, `$derived`, `$props`, `onMount`) + writable stores
- Hand-written router (`lib/router.ts`, ~14 lines), WebSocket client (`lib/socket.ts`), API client (`lib/api.ts`)
- Tailwind CSS v4 utility classes throughout
- Lucide icons via `@lucide/svelte`
- xterm.js loaded dynamically for the terminal page
- Manual i18n store (`lib/i18n.ts`)

### Target State
- Single Slint source tree (`crates/synthpanel-ui/`) built with the Rust toolchain
- All UI logic expressed in Slint's native language (`.slint`), compiled to Rust
- IPC via the existing `synthpanel-ipc` protocol (already Rust-native — no conversion needed)
- Dark/light theming via Slint's built-in color roles
- Native icon integration (Lucide rendered at compile time or via Slint's `load-font-family`)
- Terminal: xterm.js replaced by a native PTY widget (Slint-compatible alternative)
- i18n handled via Slint's localization system

---

## 2. Architecture Decision

The migration should produce a **native desktop client**, not a web replacement. Rationale:

| Concern | Web SPA (Svelte) | Native App (Slint) |
|---|---|---|
| Distribution | Served inside `synthpanel-web` binary | Bundled with `synthpanel-desktop` binary |
| Runtime | Browser + Vite build | Direct Rust execution, zero JS |
| WebSocket/HTTP | In-browser `fetch` + `new WebSocket` | Rust `reqwest` + custom reconnect loop |
| IPC access | Via REST API on localhost | Via Unix socket through `synthpanel-ipc::Client` |
| Bundle size | 57 kB gzip (terminal lazy) | ~8–15 MB native binary |
| Offline UX | Limited | Full (same network state) |

**Two-pronged outcome:**
1. New `crates/synthpanel-desktop` crate — native Slint app
2. Existing `crates/synthpanel-web/frontend/` preserved as a read-only reference during transition; removed after migration sign-off

---

## 3. Crate Layout

```
crates/
├── synthpanel-ipc/          # unchanged — already shared
├── synthpanel-core/         # unchanged
├── synthpanel-web/          # keep serving the SPA until deprecated
├── synthpanel-cli/          # unchanged
├── synthpanel-installer/    # updated to bundle desktop binary
└── synthpanel-desktop/      # NEW
    ├── Cargo.toml
    ├── src/
    │   ├── main.rs                   # entry, app lifecycle
    │   ├── ipc.rs                    # thin wrapper around synthpanel-ipc::Client
    │   ├── app_state.rs              # session, theme, locale state
    │   └── widgets/                  # reusable UI pieces
    │       ├── card.slint
    │       ├── stat_card.slint
    │       ├── status_badge.slint
    │       ├── empty_state.slint
    │       ├── sparkline.slint
    │       ├── service_list.slint
    │       ├── audit_feed.slint
    │       └── app_shell.slint       # sidebar + header layout
    ├── ui/
    │   ├── app.slint                 # root — routes to pages
    │   ├── login.slint               # auth screen
    │   ├── dashboard.slint           # metrics, services, audit
    │   ├── sites.slint               # site list
    │   ├── site_detail.slint         # per-site detail tabs
    │   ├── databases.slint
    │   ├── php.slint
    │   ├── security.slint
    │   ├── backups.slint
    │   ├── cron.slint
    │   ├── logs.slint
    │   ├── files.slint
    │   ├── deployments.slint
    │   ├── alerts.slint
    │   ├── terminal.slint            # PTY-based terminal
    │   ├── users.slint
    │   └── settings.slint
    └── build.rs                    # optional: asset bundling
```

---

## 4. Phase Breakdown

### Phase 0 — Foundation (Week 1)

**Goal:** get a compilable skeleton that connects to the live API.

1. Create `crates/synthpanel-desktop/Cargo.toml`:
   ```toml
   [package]
   name = "synthpanel-desktop"
   version.workspace = true
   edition.workspace = true

   [dependencies]
   synthpanel-ipc.workspace = true
   tokio.workspace = true
   reqwest = { workspace = true, features = ["json"] }
   serde.workspace = true
   serde_json.workspace = true
   tracing.workspace = true
   slint = { version = "1.9", default-features = false, features = ["rust-backend"] }
   anyhow.workspace = true
   ```

2. Write `build.rs` that invokes `slint-cargo-loader` to generate Rust structs from `.slint` files.

3. Create minimal `ui/login.slint` with email + password fields and a "Sign in" button.

4. Wire `ipc.rs`: a `DesktopClient` struct that wraps `synthpanel_ipc::Client` and exposes typed methods (`login`, `me`, `logout`).

5. `main.rs`: instantiate the Slint window, connect the login button to the IPC call, display the result.

6. Add `synthpanel-desktop` to the workspace `Cargo.toml` members.

**Exit criterion:** a running binary that authenticates against a live panel instance.

---

### Phase 1 — Core Shell & Navigation (Week 1–2)

**Goal:** sidebar layout, dark/light toggle, and working navigation between a couple of pages.

1. `app_shell.slint` — sidebar with 15 nav items matching current links, active-state highlighting, top bar with user avatar/name/role, theme toggle, logout button.

2. `app.slint` — the root file that holds the shell and swaps the content area based on a `current_page` enum property.

3. Port the router logic from `lib/router.ts` into a Slint enumeration:
   ```slint
   enumeration Page { Dashboard, Sites, Databases, PHP, Security, Backups, Cron, Logs, Files, Deployments, Alerts, Terminal, Users, Settings }
   ```

4. Implement the session-aware gate (Login → AppShell pattern) in `app.slint`.

5. Port `stores.ts` equivalents into `app_state.rs`:
   - `session: Option<User>`
   - `theme: Theme` (light/dark) persisted to config file instead of localStorage
   - `locale: Locale` persisted similarly

6. Port `format.ts` utilities — simple free functions in Rust, called from `.slint` via `callback`s or a shared module.

**Exit criterion:** clicking nav items switches pages; theme toggle works persistently.

---

### Phase 2 — Dashboard (Week 2)

**Goal:** port the most data-rich and interactive page first (highest confidence anchor).

1. `dashboard.slint` layout:
   - Four stat cards (CPU, memory, disk, network)
   - Two sparkline charts (CPU + memory, last 24h)
   - Service health table
   - Recent audit feed
   - Quick-action buttons

2. Port components:
   - `StatCard.slint` — accepts label, value, hint, icon slot
   - `Sparkline.slint` — takes a slice of f32 values, renders an SVG-like path via Slint's canvas
   - `ServiceList.slint` — iterates services, shows status badge
   - `AuditFeed.slint` — table of recent audit entries
   - `Card.slint` — generic bordered container with title/subtitle

3. Handle real-time updates:
   - Poll `/api/v1/metrics/snapshot` and `/api/v1/metrics/range?range=24h` every 5 seconds via a Slint timer callback
   - Maintain a sliding buffer of points in Rust (`Vec<Snapshot>`)
   - Pass the buffer to the Sparkline component

4. Port `connectSocket` equivalent: a dedicated async task in `ipc.rs` that maintains a WebSocket connection and pushes snapshots into the app state via a Slint `callback`.

**Exit criterion:** the dashboard loads, auto-refreshes, and matches the current SPA visually.

---

### Phase 3 — Forms & Data Pages (Week 3–4)

**Goal:** port the CRUD-heavy pages with form validation and tables.

Port in priority order (by interaction complexity):

| Priority | Page | Key Challenges |
|---|---|---|
| 1 | Sites (list + detail) | Table with sorting/filtering, tabbed detail view |
| 2 | Settings | Multiple sections, toggle switches, text inputs |
| 3 | Users | Table with create/edit modal |
| 4 | Security | Firewall rules table, fail2ban jail list, SSH key list |
| 5 | Databases | Create database/user forms, privilege selectors |
| 6 | Backups | Schedule editor, backup list with verify/restore actions |
| 7 | Cron | Job list, schedule expression editor |
| 8 | Deployments | Git repo URL input, run history table |
| 9 | Alerts | Channel/rule forms, test button |
| 10 | PHP | Version selector, pool metrics |

**Shared patterns to implement early:**
- `FormSheet.slint` — a reusable modal/drawer with title, fields, save/cancel buttons
- `ConfirmDialog.slint` — destructive action confirmation
- `StatusBadge.slint` — green/yellow/red dot with label

Each page follows the same shape:
```
fetch data on load → display in table/cards
form submit → POST/PATCH via IPC → invalidate cache → refresh display
error → show inline toast/banner
```

**Exit criterion:** all 14 data pages render correctly and perform mutations against the live API.

---

### Phase 4 — Terminal (Week 4)

**Goal:** replace xterm.js with a native PTY-backed terminal widget.

Options evaluated:
- **Option A: `tui-rs`/`ratatui` inside Slint** — heavy coupling, not recommended
- **Option B: embed `alacritty` subprocess** — too heavy
- **Option C: use Slint's `VideoOutput` + raw canvas — manual cell rendering**
- **Option D: spawn `synthpanel-core` pseudo-terminal and forward bytes via WebSocket, render in a Slint canvas** ← chosen approach

Implementation:
1. Open a terminal session via the existing IPC (`terminal.open`)
2. Connect to the same WebSocket endpoint (`/ws/terminal/:id`) used by the SPA
3. Render received ANSI output on a Slint `Canvas` element, maintaining a virtual terminal buffer (scrollback)
4. Forward keystrokes back through the WebSocket as JSON frames
5. Handle resize events from Slint's window resize callback
6. Apply dark theme colors matching the SPA's terminal background (`#0b1220`)

This avoids pulling in xterm.js dependencies while reusing the exact same WebSocket protocol.

**Exit criterion:** a functional terminal emulator with scrollback, paste, and resize support.

---

### Phase 5 — Polish & Integration (Week 5)

1. **i18n**: migrate English dictionary from `i18n.ts` into Slint's localization files (`locales/en.slint`). Add the same message keys with `%s`/`%{} ` placeholders.

2. **Icons**: port the 15 Lucide icons used in the nav (LayoutDashboard, Globe, Database, Server, ShieldCheck, Archive, Clock, ScrollText, FolderTree, GitBranch, BellRing, TerminalSquare, Users, Settings, Activity). Options:
   - Use Slint's built-in `icon` property with SVG assets extracted from the Lucide package
   - Or use `@lucide/slينت` if an official package exists (check)
   - Fallback: hand-pick the ~15 SVGs and inline them

3. **Accessibility**: ensure all interactive elements have `focusable: true`, labels map to `accessible-name`, skip-link is present.

4. **Dark mode parity**: verify all color tokens from `app.css` have Slint equivalents. Slint's `default-color-scheme` and color role mapping handles most cases automatically.

5. **E2E verification**: compare behavior side-by-side with the Svelte SPA against a live panel instance.

**Exit criterion:** visual parity with the SPA across all pages in both light and dark modes.

---

### Phase 6 — Packaging & Ship (Week 6)

1. Add `synthpanel-desktop` to the workspace build:
   ```toml
   # crates/synthpanel-desktop/Cargo.toml — release profile matches core
   [profile.release]
   opt-level = 2
   lto = "thin"
   codegen-units = 16
   strip = true
   panic = "abort"
   ```

2. Update `packaging/build-deb.sh` to also build and bundle the desktop binary.

3. Update `packaging/install.sh` to offer a desktop package install option.

4. Deprecation flag: mark the web frontend as "legacy" in README and docs; point users to the desktop app.

5. Remove `crates/synthpanel-web/frontend/` after migration acceptance testing passes (or keep as fallback).

---

## 5. Component Mapping Reference

| Svelte Component | Slint Equivalent | Notes |
|---|---|---|
| `App.svelte` | `app.slint` | Root — session check + shell wrapper |
| `AppShell.svelte` | `widgets/app_shell.slint` | Sidebar + header + `<output>` |
| `Card.svelte` | `widgets/card.slint` | Generic bordered container |
| `StatCard.svelte` | `widgets/stat_card.slint` | Icon + value + hint |
| `StatusBadge.svelte` | `widgets/status_badge.slint` | Dot + label |
| `EmptyState.svelte` | `widgets/empty_state.slint` | Title + description + CTA |
| `Sparkline.svelte` | `widgets/sparkline.slint` | Canvas-based line chart |
| `ServiceList.svelte` | `widgets/service_list.slint` | Table of unit statuses |
| `AuditFeed.svelte` | `widgets/audit_feed.slint` | Time-line of audit entries |
| `routes/Dashboard.svelte` | `ui/dashboard.slint` | Aggregates all dashboard widgets |
| `routes/Login.svelte` | `ui/login.slint` | Email/password + 2FA |
| `routes/Sites.svelte` | `ui/sites.slint` | Site list table |
| `routes/SiteDetail.svelte` | `ui/site_detail.slint` | Tabbed detail view |
| `routes/SiteDetailRoute.svelte` | *(merged into site_detail)* | Router redirect becomes direct navigation |
| `routes/Databases.svelte` | `ui/databases.slint` | DB list + create form |
| `routes/Php.svelte` | `ui/php.slint` | PHP version manager |
| `routes/Security.svelte` | `ui/security.slint` | Firewall/fail2ban/SSH keys |
| `routes/Users.svelte` | `ui/users.slint` | User CRUD |
| `routes/Backups.svelte` | `ui/backups.slint` | Backup list + schedules |
| `routes/Cron.svelte` | `ui/cron.slint` | Cron job editor |
| `routes/Logs.svelte` | `ui/logs.slint` | Log viewer with filters |
| `routes/Files.svelte` | `ui/files.slint` | File manager |
| `routes/Deployments.svelte` | `ui/deployments.slint` | Deployment list |
| `routes/Alerts.svelte` | `ui/alerts.slint` | Alert channels/rules |
| `routes/Terminal.svelte` | `ui/terminal.slint` | PTY terminal widget |
| `routes/Settings.svelte` | `ui/settings.slint` | Panel settings |

---

## 6. Key Technical Decisions

### 6.1 State Management
Svelte runes (`$state`) are replaced by Slint's reactive properties (`property <T>`) plus Rust-side model objects. Complex lists are held in Rust structs implementing `Model` trait for Slint table views.

### 6.2 Networking
Instead of `fetch()` + `WebSocket`, the desktop app uses:
- `reqwest` for REST calls (with session cookie jar)
- `tokio_tungstenite` or `async-tungstenite` for WebSocket connections
- A dedicated async task per long-lived connection (dashboard stream, log tail, terminal)

### 6.3 Routing
Slint does not have a concept of URLs. Navigation is driven entirely by an `enumeration Page` property on the root component. Deep-linking via command-line argument is supported; browser-style history is not applicable.

### 6.4 Styling
Tailwind utility classes map to Slint's layout and style primitives:
- `flex`, `grid` → Slint layout containers (`Row`, `Column`, `Grid`)
- `p-*`, `m-*`, `gap-*` → `padding`, `spacing`, `margin` properties
- `rounded-*`, `border-*`, `shadow-sm` → explicit style declarations
- `bg-*`, `text-*` → color properties
- `dark:` variants → conditional style overrides or dual-theme definitions

### 6.5 The Terminal
The SPA's xterm.js dependency (~68 kB gzip) is the heaviest single asset. In Slint, terminal emulation must be implemented manually. The chosen approach—forwarding raw ANSI bytes through the existing WebSocket and rendering them on a Slint Canvas—is the lowest-effort path that preserves feature parity without introducing a new runtime dependency.

---

## 7. Risk Register

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Slint Canvas performance with high-frequency terminal output | Medium | High | Profile early; consider buffering and throttling redraws |
| Lack of `@lucide/sl int` icon package | Low | Low | Extract the 15 needed SVGs manually; maintain as asset files |
| Slint `ListModel` doesn't match Svelte array reactivity | Medium | Medium | Wrap models in a Rust `ObservableVector<T>` adapter |
| Desktop app bypasses CSRF protection assumed by the SPA | Low | Medium | Cookie-based auth still applies; API server rejects missing/stale cookies |
| WebSockets from a non-browser origin trigger CORS issues | Medium | High | The panel WebSocket server must allow the desktop app's origin (or null origin); configure CORS in `synthpanel-web/src/ws.rs` |
| Binary size bloat from Slint runtime | Low | Medium | Slint's Rust backend is ~2 MB; well within acceptable range |

---

## 8. Acceptance Criteria

The migration is complete when:

1. All 14 functional pages in the Slint app match the SPA in layout and behavior
2. Every API endpoint used by the SPA is accessible from the desktop app
3. Real-time updates (dashboard metrics, log tail, terminal) work identically
4. Dark and light themes are visually consistent
5. The desktop binary builds via `cargo build --release -p synthpanel-desktop`
6. The `.deb` package includes the desktop binary and the workspace CI validates it
7. E2E tests exercise the desktop app against a live panel instance
8. The Svelte frontend is documented as deprecated in `README.md` and `PLAN.md`
