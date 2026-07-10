# Pointbreak Review inspector — design system

Source for the `pointbreak-review-inspector-ds` [Claude Design](https://claude.ai/design) gallery and
the tokenized status palette consumed by the inspector's `../assets/app.css`.

The gallery is a shared tokens component/state preview for critical inspector surfaces; it is not a full live-app mirror.
It keeps representative static states for status/readback/diff/shell/feedback
review, while runtime behavior such as routing, localStorage, copy-to-clipboard, and lazy rendering
stays in the live inspector.

## Layout

| Path | Role |
| --- | --- |
| `ABOUT.md` | Product context for Claude Design (what Pointbreak Review/`shore inspect` is, the design language, UI vocabulary). Synced to both projects alongside the cards. |
| `../assets/tokens.css` | Review's live token source and single source of truth for the palette (the only `:root`). |
| `styles.css` | Component rules only — references the tokens via `var(--…)`. |
| `_bodies/*.body.html` | Per-card markup fragments (the authored content of each card). |
| `variants/instrument-neutral.css` | Temporary, unserved surface/text/accent candidate for the Review comparison study. |
| `<group>/<card>.html` | **Generated, git-ignored.** Run the baker to produce them. |
| `contrast-check.mjs` | Product-local text-contrast audit of record; parses the live token source for both themes. |
| `pointbreak-brand.lock.json` | Immutable source commit, manifest digest, and local destinations for vendored brand assets. |
| `brand-check.mjs` | Offline verification of every locked local byte digest and SVG geometry digest. |
| `logo/pointbreak-logo.svg` | Original multiband logo for gallery identity evidence only; live compact chrome remains mono. |
| `comparisons/*.html` | **Generated, git-ignored.** Matched baseline/candidate cards for the Review visual study. |
| `_bodies/bake.sh` | Verifies brand and contrast contracts, then bakes self-contained cards from a fragment + tokens + `styles.css`. |

Each baked card is self-contained: the baker prepends the
`<!-- @dsCard group="…" -->` marker the gallery indexes on, inlines
`../assets/tokens.css` then `styles.css`, then appends the body fragment. Cards
are grouped as **Foundations, Navigation, Inputs, Data, Feedback**. The
Foundations card (`_bodies/foundations.body.html`) carries its own swatch JS and
reads the inlined token values via `getComputedStyle`.

The Data group includes a static mirror of the shipped Attention lens. The
`instrument-neutral` foundations pair is an explicitly unshipped comparison
artifact: the baker composes it after live tokens, but no inspector asset or
route loads the variant.

The comparison matrix covers foundations, wide/narrow navigation, timeline,
Attention, review facts, annotated diff, compact density, and large identity in
both themes. Run the audit against the current product or the isolated candidate
with:

```sh
node contrast-check.mjs
node contrast-check.mjs --variant variants/instrument-neutral.css
```

Both commands keep the live token file read-only. The expanded light-theme
syntax-on-tinted-row matrix is reported separately as decision evidence rather
than silently changing the product palette during a gallery comparison.

## Workflow

1. Edit `styles.css` and/or a `_bodies/*.body.html` fragment.
2. Regenerate the cards; the baker runs the offline brand check and local
   contrast audit before writing generated output:
   ```sh
   bash _bodies/bake.sh
   ```
3. Sync to claude.ai/design via the DesignSync tool / `/design-sync` skill
   (project `pointbreak-review-inspector-ds`): `list_files` → `finalize_plan` → `write_files`.

The palette is single-sourced in `../assets/tokens.css` (the served frontend's
only `:root`); `bake.sh` inlines that same file into every card, so the gallery
and the live inspector resolve the same shared tokens. Add or rename a token in
`tokens.css` and re-bake.

Node is design tooling only. Neither Node nor this gallery participates in the
Rust build/runtime, and no product build or audit depends on the marketing
repository or a sibling `pointbreak-brand` checkout. Brand updates land in the
central repository first, then Review deliberately vendors the selected files
and updates `pointbreak-brand.lock.json`; normal verification reads only that
lock and committed local assets.
