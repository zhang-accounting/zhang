# Frontend design notes

## Stack
React 19 + Vite 5 + TypeScript · Tailwind CSS 4 (`@tailwindcss/vite`; no `tailwind.config`/PostCSS) · shadcn CLI 4.x, style
`base-nova` on **Base UI** (`@base-ui/react`) · lucide-react · next-themes (class) · sonner · recharts 3 (`ui/chart`) ·
react-day-picker 10 (`ui/calendar`) · react-i18next · jotai. `cn` comes from `@/lib/utils` (re-exports the `cn` package).

## Tokens (`src/global.css`)
- Natural, mostly white / grey look modelled on 多少记账 (hex, Radix Sage neutrals + the logo turquoise `#04ccb1`, logo in
  `public/otter-*.png`): `background` page, white `card`s, slightly darker `sidebar`, `foreground` text, `foreground-2`
  secondary text (nav labels, account rows, chips), `muted-foreground` for hints (kept at 4.5:1), grey `muted` (chips,
  secondary), neutral `accent` (hover), `track` (empty part of bars). The brand colour appears as the active-nav tint
  (`sidebar-accent` / `sidebar-accent-foreground`) and as fills. Use semantic classes only: `bg-background text-foreground`,
  `text-foreground-2`, `text-muted-foreground`, `bg-card`, `border`, `bg-primary text-primary-foreground`, `text-link`,
  `text-destructive`, `bg-muted`, `bg-track`. No Tailwind palette classes (`emerald-500`,
  `gray-*`, `white`) or raw hex in TSX; they ignore the theme and break dark mode.
- `primary` is a **fill** colour only: primary buttons, the new-transaction button, `sidebar-primary`, checked switches,
  progress fills, `bg-primary/10` icon tiles. It is ~2:1 on the light surfaces, so never use it for text, icons or thin
  lines (`text-primary`, `border-primary`, `stroke` of a chart line): use `link` (`text-link`, `border-link`) for brand-
  coloured text, active nav / tab icons and labels, links, focus and drag accents. `<Button variant="link">` needs
  `className="text-link"` (the CLI variant uses `text-primary`).
- Money: amounts are regular weight and tabular (`<Amount>`); only group totals (sidebar) are semibold. `<Amount plain>` drops
  the currency symbol for dense lists where it is implied (sidebar account rows).
- Status: `text-positive` / `text-negative` for signed money (`<Amount tone>`), `warning` (`bg-warning`, `bg-warning/10
  text-warning`) for flagged / unsaved / disconnected, `destructive` (= `negative`) for errors.
- Charts: income `var(--chart-1)` (teal), expenses `var(--chart-2)` (caramel) everywhere; single-series lines / bars
  `var(--chart-1)`; negative values `var(--negative)`; more categories `chart-1..5` in order, never cycled. Text on chart
  fills uses `var(--background)`. Never `hsl(var(--chart-2))`.
- Fonts: `font-sans` = system UI stack with CJK fallbacks (PingFang SC, Microsoft YaHei, Noto Sans CJK); `font-mono`.
  Amounts: `tabular-nums`. Radius: `--radius` 0.25rem (data-dense, crisp corners) → `rounded-md/lg/xl`; keep
  `rounded-full` for dots, avatars / round icon tiles, spinners and progress tracks, and pills only for chips <= 24px tall.
- Dark mode: `.dark` on `<html>` via next-themes (`theme` in localStorage); `index.html` applies it before first paint.

## Shell (`src/layout`)
- `AppShell` = `SidebarProvider` → `AppSidebar` (>= md, `collapsible="icon"`, Ctrl/Cmd+B) + `SidebarInset` (`TopBar` + content
  column `max-w-7xl px-4 md:px-7`) + `MobileTabBar` (< md, 4 tabs + "More" sheet) + `NetworkStatus`.
- **No desktop top bar.** `TopBar` is mobile-only: the otter + ledger title on top-level pages, a back link + page title on
  nested pages (from `breadcrumbAtom`), and the 40px turquoise "+" (`<NewTransactionButton variant="icon">`). On desktop
  `PageHeader` renders the breadcrumb trail (`Accounts › Assets:WeChat`) above the `<h1>` of nested pages. Pages do not set it:
  `PageMeta` in `router.tsx` sets the trail and the document title from the page's `ROUTES` entry (its section link, then
  the `:param` or label of the page), so a new page gets both by declaring its route.
- `AppSidebar` anatomy (top to bottom): header (otter 28px, ledger title, `Zhang <version>` + online dot / label, muted reload
  button) · full-width "New transaction" card button (`variant="sidebar"`, plus in `link`) · primary nav
  (`SIDEBAR_PRIMARY_LINKS`: Overview, Journals, Report, Balance sheet, Budget) + collapsible "More" (`SIDEBAR_MORE_LINKS`,
  state in localStorage `sidebar-more-open`, auto-open on its routes) · `SidebarAccounts` · update notice · footer (border-top:
  Tools, Settings, then theme / language / sign-out (auth on) / collapse icon buttons). Nav items are 34px: `foreground-2`
  label, muted icon, neutral hover; active = `sidebar-accent` tint + `sidebar-accent-foreground` + medium + `link` icon. Icon
  mode keeps the otter, a 32px "+" and icon-only items with tooltips; the accounts list is hidden.
- `SidebarAccounts` (Actual Budget style): "Accounts" label + search toggle (inline filter), "All accounts" net total, Assets /
  Liabilities subtotals (semibold, with symbol), then every open account (name without the top-level type, `title` = full
  name, balance never truncated, without symbol, negatives `text-negative`). Rows are 28px / 13px links to
  `/accounts/<name>`; the current account is tinted. The list scrolls on its own; Income / Expenses are not listed.
- Routes/menus live in `nav-links.ts` (`*_LINK`, `SIDEBAR_*_LINKS`, `MOBILE_PRIMARY_LINKS`, `MOBILE_MORE_LINKS`, `isLinkActive`).
  `shortLabel` is the tab-bar label (`报表` vs the sidebar's `统计报表`); `/accounts` is "Balance sheet" in the sidebar and
  "Accounts" in the tab bar.
- Controls: `OnlineStatus` (browser + SSE `onlineAtom`), `ThemeToggle` / `LanguageSwitch` (`className`, `side`), `useReloadLedger()`.

## Auth gate and login page (`layout/AuthGate`, `pages/Login`)
- `App` = `AuthGate` → `LedgerApp` (`AppShell` + routes + `useServerEvents`). The gate loads `GET /api/auth/status` into
  `authStateAtom` (`states/auth`; splash while loading, retry screen if the server cannot be reached, a 404 = old server = no
  auth). With `enabled && !authenticated` it renders the login page at `/login?next=<path>` instead of the shell (open-redirect
  safe `returnPath`), so the SSE stream and the ledger atoms only exist while signed in; `resetLedgerStateAtom` drops the cached
  ledger data once the shell has unmounted. Explicit sign-out goes to `/login` without `next`.
- Session expiry: the server answers `401 { message }` (no `WWW-Authenticate`, no browser popup). The fetcher middleware and
  `responseError` (plain `fetch`) report 401s outside the sign-in calls (`isSignInUrl`) → `signedOutAtom('expired')` → login
  page with a "session expired" note; a quiet status refresh never leaves the login page (only a sign-in does). A permanently
  closed SSE stream re-checks the status. All `/api` calls are same-origin (`apiBaseUrl`, the dev server proxies `/api`) so the
  HttpOnly cookie is sent; auth calls live in `api/auth.ts` (`credentials: 'same-origin'`).
  `<Toaster>` is mounted inside `LedgerApp` (no toasts over the login page); sign-in dismisses the previous session's toasts.
- Login page (`AuthScreen`: full screen, no shell, theme + language buttons at the bottom): otter 48px, "Sign in to <title>"
  (status `title`, fallback "Zhang") + muted subtitle, then one card (`max-w-sm`): passkey registered → 44px primary "Sign in
  with passkey" (also on Enter outside a control); passkey on but none registered → "Set up a passkey" form (`ZHANG_PASSKEY`
  secret + optional name, default "<browser> on <OS>"); password on → username / password form under an "or" divider, its
  button primary only when it is the only method (outline next to the passkey button). Each action has a muted hint line
  under it that turns into the red error (same slot: no layout shift). No WebAuthn (insecure context, old browser, IP-address
  host — never a valid RP ID) → a note, and the password form becomes primary.
- WebAuthn: `lib/webauthn` converts the webauthn-rs JSON (`{ publicKey }`, base64url) both ways (unit-tested);
  `lib/passkey` runs start → `navigator.credentials` → finish and throws `PasskeyError` kinds that
  `usePasskeyErrorMessage` turns into one sentence (`NotAllowedError` → "The passkey prompt was dismissed.").
- Signed in with auth on (`canSignOutAtom`): "Sign out" icon button in the sidebar footer row and a button in the mobile More
  sheet (`useSignOut`). Passkey mode: Settings → Passkeys (`components/auth/PasskeySettings`) lists, adds (no secret) and
  removes passkeys; the server's 409 (last passkey without password sign-in) is shown in the confirm dialog.

## Journal rows (`components/journalLines/JournalRow.tsx`)
- Used by Journals and the overview's recent activity. Journals are grouped by day: heading `Sep 16` (semibold; the year is
  added outside the current year) + muted weekday, then one bordered card of rows.
- Row: line 1 narration (14px) + payee (13px muted); line 2 account chips (12px, `bg-muted`, `rounded-md`) `own → destination`
  with a red arrow for money out, `own ← source` with a green arrow for income, neutral for transfers, `+N` for more postings
  (one line, chips truncate, the destination first), then tag / link chips and the flagged / unbalanced badges. Right: amount
  (15px regular, `tone`) with the time (or day + time, `showDate`) under it, then the "…" menu (hover / focus on desktop,
  always on touch). Balance checks / pads show their type, account chip(s) and the resulting balance. The whole row opens the
  preview. `dense` (side cards): 13px title, no menu (the preview has the actions).

## Overview (`pages/Home.tsx`)
- Header (title, 30-day window, "Open report") · 4 KPI cards (income `positive`, expenses `negative`, regular 20px values) ·
  grid `2fr / 1fr` from lg: net worth | `MonthBudgetsCard`, income & expenses | recent activity (+ ledger health line: the
  otter + "healthy", or "N errors" opening the `ErrorBox` dialog). Charts grow with their row.
- `MonthBudgetsCard`: the month of the window end, up to five open budgets with the most activity: name, `activity /
  assigned`, a 6px `bg-track` bar (`bg-primary`, `bg-negative` once over budget, capped at 100%), "Used x%" + "Left ¥y" or
  "Over by ¥z" + the uncapped percentage; header right `<Month> · Left ¥total`; footer "View all" → `/budgets?year=&month=`.
  No budgets: one muted line + a link to the budget docs.

## Page primitives (`import { … } from '@/components/layout'`)
- `PageShell` `{ width?: 'default' | 'narrow', ...div }` – page root, vertical rhythm (`gap-4 md:gap-6`).
- `PageHeader` `{ title, description?, actions?, children?, className? }` – the page `<h1>`; actions wrap under it on mobile.
- `EmptyState` `{ icon?: LucideIcon | ReactNode, title, description?, action?, className? }`.
- `ResponsiveList<T>` `{ items, getKey, columns: { key, header, cell(item, i), className? }[], renderCard(item, i),
  getItemHref?, linkColumn?, onItemClick?, loading?, empty?, className? }` – table >= md, stacked tappable cards < md.
  Rows that navigate use `getItemHref` (real `<Link>`s: keyboard, screen readers, Cmd/middle-click): desktop puts a stretched
  link in the `linkColumn` cell (default: first column), mobile makes the whole card the link. Don't put other links inside
  such rows. `onItemClick` is only for in-page actions (open a preview / lightbox).
- `RefreshingLabel` – "Updating…" + spinner (`role="status"`). While a page refetches for a new range / month and still shows
  the old numbers, use it as the `PageHeader` description and put `aria-busy` + `opacity-60` on the stale content.
- `SheetCloseButton` (`layout/SheetCloseButton`) – translated 40px close button; use with `<SheetContent showCloseButton={false}>`.
- `useIsMobile()` – `true` below 768px (initialised synchronously, no flash).

```tsx
<PageShell>
  <PageHeader title={t('NAV_ACCOUNTS')} actions={<Button>…</Button>} />
  <ResponsiveList items={rows} getKey={(r) => r.name} columns={cols} renderCard={(r) => <AccountCard row={r} />}
    empty={<EmptyState icon={WalletMinimal} title="No accounts" />} />
</PageShell>
```

## Responsive rules
- Mobile first; the breakpoint that matters is `md` (768px): sidebar ↔ tab bar, table ↔ cards. Verify at 390px.
- No horizontal page scroll at 390px: lists of records use `ResponsiveList` cards on mobile; any remaining table must sit in
  `ui/table` (it scrolls inside its own container). Use `min-w-0` + `truncate` for long account names.
- Tap targets >= 40px on mobile: shadcn buttons are desktop-dense (`h-8`), so give mobile-facing actions `h-10 md:h-8`
  (or `size="lg"`), icon buttons `size-10 md:size-8`. `global.css` also enforces it below md for every `[data-slot=button]`
  (40×40 minimum), toggle, input, select trigger, input group (control + inline addon buttons fill the full 40px), tab,
  accordion trigger and sheet / dialog close button. It is unlayered on purpose (beats the primitives' utilities); >= md is
  untouched. Elements that are not one of those slots (plain `<button>`, `<a>`) still need explicit `h-10 md:h-…`.
- Exception: inline `#tag` / `^link` chips inside journal rows stay compact but are at least 24×24 (`h-6 min-w-6`), the
  WCAG 2.5.8 minimum; the row itself remains the large target.
- Text inputs / textareas are 16px on mobile (`text-base md:text-sm`) so iOS does not zoom on focus. Scrollable sheets and
  drawer bodies use `overscroll-contain`. Animations respect `prefers-reduced-motion` (`motion-safe:` / `motion-reduce:`).
- Never `inputMode="decimal"` on amount fields: the iOS keypad has no `-` and no letters (`-21.50 CNY`).
- Fixed bottom UI must clear the tab bar: `bottom-[calc(4rem+env(safe-area-inset-bottom))] md:bottom-0`.

## Data entry, errors and navigation
- Reading ledger data: shared resources are atoms in `states/` that read `ledgerRevisionAtom`, page data goes through
  `useLedgerQuery(query, deps)` (`states/ledger`). SSE `Reload`, every write and the sign-out reset call `ledgerChanged()`
  once, so every open page follows the ledger. Lint keeps `react-use`'s `useAsync*` inside `states/ledger.ts`; the raw
  editor's file content is the one exception, as a reload must never overwrite unsaved text.
- Request failures: `toast.error(title, { description: await apiErrorMessage(error) })` (`lib/api-error`) shows the server's
  `{ message }` (generated client `ApiError`, `Response`, `Error`, string). Plain `fetch` uploads throw
  `await responseError(response)`.
- `api/fetcher.ts` rewrites the `tags` / `links` query params to `tags[]=…`: zhang-server rejects `tags=…` (400) and ignores
  `tags%5B%5D=…`.
- Transactions: the update API rebuilds every posting from `{ account, unit, metas }`, so cost / price / posting comments /
  posting flags are dropped. `transactionEditBlocker()` (`journalLines/journal-utils`) disables "Edit" (row menu + preview) for
  transactions with a cost or with postings in several commodities (price) and points to Raw Edit; comments and posting
  flags are not in the journal payload, so saving an edit asks for confirmation. `TransactionEditForm` accepts only
  `<number> <COMMODITY>` per amount and its preview is rendered from the exact request body (pure helpers and tests in
  `components/transaction-form-utils`).
- Metadata (`metas: { key, value }[]` on transactions and postings): the form edits it as key / value rows (transaction
  section; per posting behind a collapsed toggle with a count badge, as an indented block under the row). Key inputs disable
  auto-capitalisation; key rules are the server's (400 `{ message }` → the usual toast). The preview lists transaction metas
  under "Details" and each posting's metas as a small indented key / value list under that posting.
- `document` metas are files, not editable metadata: the server links a posting's `document` to its transaction (older
  uploads after the last posting of a `.bean` ledger are posting metadata), so the preview grid and the row indicator
  (`transactionDocuments()`, each path once) and the form (hidden, sent back unchanged) treat posting and transaction documents
  alike; the posting's metadata list leaves them out. The form's text preview follows the ledger format from `/api/files`
  (first file `.bean`: date + `time` meta, beancount quoting).
- Unsaved changes: `useUnsavedChangesGuard(dirty, message)` covers tab close / reload (`beforeunload`) and in-app links (a
  capture-phase click listener on `a[href]` asks first). The app uses `<BrowserRouter>`, so `useBlocker` is unavailable and
  **browser Back cannot be blocked**; keep `?file=`-style switches on `replace` so Back leaves the page instead of silently
  swapping content. Programmatic `navigate()` calls are not guarded either.
- Global UI atoms that open modals (`previewJournalAtom`, `editTransactionAtom`) are cleared when the route changes.
- `<html lang>` follows the UI language (`en` / `zh-CN`); dates go through `useDateFormat()` (never bare date-fns `format`
  with month names); document titles and toasts are translated.

## Base UI vs the old Radix API
- No `asChild`: compose with `render`, e.g. `<PopoverTrigger render={<Button variant="outline" />}>Pick</PopoverTrigger>`.
  Never nest a `<Button>` inside a trigger (nested `<button>` error).
- Links that look like buttons: `<Link className={cn(buttonVariants({ variant: 'ghost' }), 'extra classes')}>`. Always wrap
  in `cn`: `buttonVariants({ className })` concatenates without resolving conflicts (e.g. `size-8` vs `size-10`).
- `Select`: pass `items={[{ value, label }]}` so `SelectValue` shows the label; `onValueChange(value: string | null)`.
- `Accordion`: no `type`/`collapsible` (use `multiple`). `Calendar`: `autoFocus` (not `initialFocus`).
- `ui/combobox` is Base UI's combobox; the grouped account picker is `components/basic/GroupCombobox`.
- `ui/drawer` is Base UI's drawer (no vaul). `ui/auto-drawer` (custom) = Dialog >= md, Drawer < md.
- Mantine hooks were replaced by `src/hooks/*` (`use-local-storage`, `use-disclosure`, `use-list-state`,
  `use-document-title`, `use-debounced`, `use-input-state`) and `react-use`.

## Adding a shadcn component
1. From `frontend/`: `npx shadcn@latest add <name>` (docs: https://ui.shadcn.com/docs/components/base/<name>).
2. `pnpm run prettier:fix` (CLI output uses double quotes), then `pnpm run build`.
3. `src/components/ui/*` is CLI-owned: do not hand-edit. Known local delta to re-apply after `--overwrite`:
   `hooks/use-mobile.ts` (synchronous initial state).
