# Zhang documentation

The source of <https://zhang-accounting.kilerd.me>, built with [Astro](https://astro.build) and
[Starlight](https://starlight.astro.build).

## Running the site locally

You need Node.js and pnpm 9. Without pnpm, prefix each command with `npx --yes pnpm@9` instead of `pnpm`.

```shell
cd docs
npx --yes pnpm@9 install
pnpm dev      # serves the site at http://localhost:4321 and reloads on changes
pnpm build    # runs `astro check`, then builds the static site into dist/
```

Run `pnpm build` before you open a pull request: it checks the project with `astro check` and renders every page, so
it catches errors that `pnpm dev` only shows when you open the broken page.

## Where pages live

- English pages are in `src/content/docs/`. The file `guides/budgets.md` is served at `/guides/budgets/`.
- Simplified Chinese pages are in `src/content/docs/zh-cn/` and mirror the English tree with the same file names, so
  `zh-cn/guides/budgets.md` is served at `/zh-cn/guides/budgets/`. A page without a translation falls back to the
  English one. When you add or move an English page, add or move its Chinese counterpart at the same path.
- Images go in `src/assets/`.

Every page has a `title` and a `description` in its frontmatter, and a `sidebar: { order: N }` that orders it
within its group. Quote a `description` that contains `: `.

## How the sidebar is built

The sidebar is defined in `astro.config.mjs`:

- **Getting Started**, **Guides** and **Developers** list their folders automatically, in `sidebar.order` order.
- **Deployment** and **Reference** contain an automatic group for a nested folder (`deployment/data-sources/`,
  `reference/directives/`), and list the pages next to it by slug (`deployment/authentication`,
  `deployment/upgrading`, `reference/query-language`, `reference/error-codes`). A new page directly in
  `deployment/` or `reference/` must be added to that list, or it does not appear in the sidebar.
- Group labels have a `translations` entry for `zh-CN`. Translate the label of any group you add.

## Moving a page

Old URLs must keep working: links from the README, from the web UI of older releases and from search engines point to
them. When you move or rename a page, add the old slug and the new one to `movedPages` in `astro.config.mjs`. It
creates the redirect for the English page and for its `/zh-cn/` counterpart. Then update the links to the page.

## Writing pages

- Link to other pages with absolute paths and a trailing slash, such as `[balance](/reference/directives/balance/)`.
  Chinese pages link to the `/zh-cn/` pages. An anchor is the slug of a heading, such as `#file-permissions`.
- Write ledger examples in a ```` ```zhang ```` code block, which is highlighted as beancount (see
  `expressiveCode` in `astro.config.mjs`), and commands in ```` ```shell ````.
- Check that every ledger example loads in Zhang without unexpected errors: put it in a `main.zhang`, start
  `zhang serve` on it and look at the error list.
- Use Starlight asides (`:::note`, `:::tip`, `:::caution`, `:::danger`) in Markdown. Components such as `Card`,
  `Tabs` or `Steps` need an `.mdx` file and an import from `@astrojs/starlight/components`.
- Describe the behavior of the `main` branch, and check it against the code or by running Zhang.
- CI checks the spelling of the whole repository with [typos](https://github.com/crate-ci/typos).

## Deployment

The `docs` job of `.github/workflows/build-latest.yml` builds the site on every pull request, and a failed build
blocks the merge. On pushes to `main` and `develop` it also deploys the site to Cloudflare Pages (project
`zhang-docs`).
