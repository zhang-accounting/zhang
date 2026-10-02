# Frontend design notes

## Stack
React 19 + Vite 5 + TypeScript · Tailwind CSS 4 (`@tailwindcss/vite`; no `tailwind.config`/PostCSS) · shadcn CLI 4.x, style
`base-nova` on **Base UI** (`@base-ui/react`) · lucide-react · next-themes (class) · sonner · recharts 3 (`ui/chart`) ·
react-day-picker 10 (`ui/calendar`) · react-i18next · jotai. `cn` comes from `@/lib/utils` (re-exports the `cn` package).

## Tokens (`src/global.css`)
- Turquoise otter palette (hex, Radix Sage neutrals + the logo turquoise `#04ccb1`, logo in `public/otter-*.png`): near-white
  / near-black surfaces, neutral grey `muted` / `accent` / `sidebar-accent` (hover and active items), deep-teal `link`.
  Use semantic classes only: `bg-background text-foreground`, `text-muted-foreground`, `bg-card`, `border`, `bg-primary
  text-primary-foreground`, `text-link`, `text-destructive`, `bg-muted`. No Tailwind palette classes (`emerald-500`,
  `gray-*`, `white`) or raw hex in TSX; they ignore the theme and break dark mode.
- `primary` is a **fill** colour only: primary buttons, the new-transaction button, `sidebar-primary`, checked switches,
  progress fills, `bg-primary/10` icon tiles. It is ~2:1 on the light surfaces, so never use it for text, icons or thin
  lines (`text-primary`, `border-primary`, `stroke` of a chart line): use `link` (`text-link`, `border-link`) for brand-
  coloured text, active nav / tab icons and labels, links, focus and drag accents. `<Button variant="link">` needs
  `className="text-link"` (the CLI variant uses `text-primary`).
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
- `AppShell` = `SidebarProvider` → `AppSidebar` (>= md, `collapsible="icon"`, Ctrl/Cmd+B) + `SidebarInset`
  (`TopBar` + content column `max-w-7xl px-4 md:px-6`) + `MobileTabBar` (< md, 4 tabs + "More" sheet) + `NetworkStatus`.
- `TopBar`: breadcrumb from `breadcrumbAtom`; on mobile the last crumb is the title and the previous crumb is a back link.
  Pages keep calling `setBreadcrumb([SOME_LINK, { label, uri, noTranslate: true }])`.
- Routes/menus live in `nav-links.ts` (`*_LINK`, `NAV_GROUPS`, `MOBILE_PRIMARY_LINKS`, `MOBILE_MORE_LINKS`, `isLinkActive`).
- Controls: `OnlineStatus` (browser + SSE `onlineAtom`), `ThemeToggle`, `LanguageSwitch` (`useLanguage()`), `useReloadLedger()`.

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
- Request failures: `toast.error(title, { description: await apiErrorMessage(error) })` (`lib/api-error`) shows the server's
  `{ message }` (generated client `ApiError`, `Response`, `Error`, string). Plain `fetch` uploads throw
  `await responseError(response)`.
- `api/fetcher.ts` rewrites the `tags` / `links` query params to `tags[]=…`: zhang-server rejects `tags=…` (400) and ignores
  `tags%5B%5D=…`.
- Transactions: the update API rebuilds every posting from `{ account, unit }`, so cost / price / posting comments / posting
  flags are dropped. `transactionEditBlocker()` (`journalLines/journal-utils`) disables "Edit" (row menu + preview) for
  transactions with a cost or with postings in several commodities (price) and points to Raw Edit; comments and posting
  flags are not in the journal payload, so saving an edit asks for confirmation. `TransactionEditForm` accepts only
  `<number> <COMMODITY>` per amount and its preview is rendered from the exact request body.
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
3. `src/components/ui/*` is CLI-owned: do not hand-edit. Known local deltas to re-apply after `--overwrite`:
   `scroll-area.tsx` (unused React import removed) and `hooks/use-mobile.ts` (synchronous initial state).
