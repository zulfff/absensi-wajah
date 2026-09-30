# Design Tokens — Absensi Wajah

Source of truth: `web/src/styles/tokens.css` (three layers: primitive →
semantic → component). Every value in the app resolves through it.

## Theme

**Graphite** — neutral cool-grey ramp + one teal accent. Light and dark both
designed (dark rebalances the surface stack, drops accent chroma ~10%, replaces
shadow depth with border rings). OKLCH throughout.

## Colour

- **Neutral ramp:** `--gray-50` … `--gray-950` (cool tint, hue 240).
- **Accent ramp:** `--accent-50` … `--accent-900`, hue 195 (teal), held constant
  across the ramp.
- **Semantic bases:** `--green-*` (success), `--amber-*` (warning), `--red-*`
  (error).
- **Semantic roles (light):** `--surface-canvas`, `--surface-raised`,
  `--surface-overlay`, `--surface-sunken`, `--surface-sidebar`; `--text-primary`
  / `--text-secondary` / `--text-tertiary`; `--border-subtle` / `--border-default`
  / `--border-strong` / `--border-focus`; `--accent-bg` / `--accent-text`.
- **Kiosk overlay (theme-independent):** `--overlay-surface`,
  `--overlay-surface-strong`, `--overlay-text`, `--overlay-text-muted`,
  `--overlay-border`, `--overlay-ring`, `--overlay-ring-strong`. These do NOT
  flip with the theme — the kiosk sits on camera imagery.

## Spacing

8pt scale: `--space-xs` 4px, `--space-sm` 8px, `--space-md` 16px, `--space-lg`
24px, `--space-xl` 32px, `--space-2xl` 48px, `--space-3xl` 64px, `--space-4xl`
96px.

## Type

Sizes `--text-xs` 12 … `--text-6xl` 60. Weights 400/500/600/700
(`--font-regular`/`medium`/`semibold`/`bold`). Leading `--leading-tight` 1.15 /
`snug` 1.3 / `normal` 1.5. Tracking `--tracking-tight` -0.02em (headings and
large figures), `--tracking-wider` 0.04em (small labels only).

Font: Inter (system fallback chain). One body face; numbers use
`font-variant-numeric: tabular-nums` (the `.tnum` class).

## Radii (varied by role — never uniform)

`--radius-sm` 4px, `--radius-md` 8px (buttons, inputs), `--radius-lg` 12px
(cards), `--radius-xl` 16px (large panels/kiosk card), `--radius-full` (dots,
avatars, pills).

## Shadows

`--shadow-sm` / `--shadow-md` / `--shadow-lg`, each ≥2 layers (ambient +
direct). In dark mode shadows collapse to a border ring (shadow is invisible on
near-black).

## Motion

`--duration-fast` 150ms (hover/colour), `--duration-normal` 250ms,
`--duration-slow` 400ms (page/kiosk transitions only). `--ease-out` is the
default curve. `prefers-reduced-motion` collapses all durations globally.

## Z-index (semantic, never arbitrary)

`--z-base` 0, `--z-raised` 1, `--z-sticky` 20, `--z-modal-backdrop` 30,
`--z-modal` 40, `--z-toast` 50.

## Breakpoints & responsive

Three breakpoints, chosen where the layout breaks (not device names):

| Width | Shell | Content |
|---|---|---|
| `≤768px` | Bottom nav (thumb zone) + fixed top bar; sidebar hidden | Single column, 16px page padding |
| `769–1024px` | Icon rail (3.5rem), labels hidden | 24px padding |
| `>1024px` | Full sidebar (15rem), labels visible | 32px padding |

- **Viewport height uses `dvh`** (`100dvh` with a `100vh` fallback), not `100vh`
  — the mobile URL bar changes the visible height and clips a `100vh` layout.
- **Safe-area insets** (`--safe-*`, backed by `env(safe-area-inset-*)`) are added
  to every fixed/sticky edge: the kiosk top/bottom bars, the mobile top bar, the
  bottom nav, and the content pane's padding. Requires
  `viewport-fit=cover` in `index.html`.
- **Touch floor:** `--touch-target` (44px) applied to every control under
  `@media (pointer: coarse)`, so a mouse-only design does not shrink touch targets.
- **Thumbs-first:** primary navigation (bottom nav) sits in the thumb zone;
  destructive actions (logout) are moved to the top bar, deliberately out of the
  thumb's easy reach.
- **Tables** scroll inside `.table-scroll` (`overflow-x: auto`) with a sticky
  header; the page itself never scrolls horizontally. The table has
  `min-width: 40rem` so columns stay readable and the wrapper does the scrolling.
- **Body text never goes below 16px** on a phone (prevents iOS zoom-on-focus).

## Component tokens

`--button-radius`, `--button-px`, `--button-py`, `--input-bg`,
`--input-radius`, `--card-radius`, `--card-pad`, `--row-height` (3rem).

## Rules

- No raw hex, off-scale px, or magic z-index outside `tokens.css`. Enforced by
  ui-craft `tokens_lint` (target 100).
- Border radii vary by role; a single uniform radius on everything is a tell.
- 1px hairlines and the kiosk's light-on-overlay text are the only raw values
  permitted, and both come from tokens anyway.
