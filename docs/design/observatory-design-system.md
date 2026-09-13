# Tracepress Observatory design system

Status: Phase 4.0 implementation contract. The system optimizes for long technical investigation sessions at 1024–1600 px, with desktop at 1280 px as the primary target.

## Principles

- Dense, calm, precise developer tooling; low decoration and strong alignment.
- Measurement provenance and missingness are first-class interface data.
- Dark surfaces, subtle boundaries, minimal radius, and no gradients, glass effects, or decorative glow.
- A chart never implies precision that the underlying measurement does not provide.

## Tokens

All repeated values are CSS/Tailwind tokens in `tailwind.css` and `assets/main.css`.

| Role | Token | Value |
| --- | --- | --- |
| App background | `background` | `#0b0e12` |
| Primary surface | `surface-1` | `#11151b` |
| Raised surface | `surface-2` | `#171c24` |
| Hover surface | `surface-hover` | `#1d2430` |
| Border | `border` | `#27303d` |
| Strong border | `border-strong` | `#394658` |
| Primary text | `text-primary` | `#e6eaf0` |
| Secondary text | `text-secondary` | `#a6afbd` |
| Muted text | `text-muted` | `#717c8d` |
| Success | `success` | `#63b68b` |
| Warning | `warning` | `#d6a657` |
| Danger | `danger` | `#dc6b72` |
| Accent | `accent` | `#79a8ff` |

Status colors are never the only status signal: each colored badge includes text. Surfaces use 1 px borders. Panels use 3 px radius, controls 3 px, and compact badges 2 px. Shadows are reserved for menus/tooltips.

## Typography and spacing

Use the local system sans stack. Identifiers, hashes, token counts, request IDs, and SHAs use the system monospace stack. Body and metrics are 13 px, tables 12 px, captions 11 px, and page titles 22 px. Line height is 1.45 for prose and 1.3 in tables.

Spacing follows a 4 px base: 4, 8, 12, 16, 20, 24, and 32 px. Page gutters are 24 px at 1280 px and 16 px near 1024 px. Cards use 16 px padding; dense table cells use 8 px vertically and 10 px horizontally.

## Application shell

`AppShell` is a two-column grid: a fixed 216 px sidebar and a fluid main region. `TopBar` is 44 px high and contains the current data source, refresh action, and health state. The sidebar groups Overview; Sessions, Context, Workloads; Baselines; and disabled/coming-soon Compression. Active navigation uses an accent inset edge and raised surface, not a large pill.

At 1024 px the sidebar becomes 184 px, page gutters shrink, metric grids wrap, and tables scroll horizontally. Mobile navigation is explicitly outside the Phase 4.0 target.

## Primitives

- `Button` and `IconButton`: 30 px control height, visible border, hover surface, and 2 px accent focus ring. Icon-only actions require an `aria-label`.
- `Badge`: compact rectangular label. Variants are neutral, success, warning, danger, estimated, provider-reported, and unavailable.
- `Card`: a bordered surface with optional header. Cards do not float and are not oversized.
- `Metric` and `MetricGroup`: 11 px uppercase label, 18 px monospace value, compact provenance below. Exact value is available through `title`.
- `DataTable`: real table semantics, sticky header where useful, tabular numerals, row hover, no card-per-row pattern.
- `Pagination`: previous/next controls with current range; cursor remains opaque to the UI.
- `Tabs`, `Select`, and `FilterBar`: compact controls with persistent selected state. Filters do not hide unavailable provenance.
- `Tooltip`: concise clarification on hover and keyboard focus; never the sole location of essential quality warnings.
- `Skeleton`: muted, non-flashing blocks that preserve final layout.
- `EmptyState`: title plus a factual explanation; no illustration required.
- `ErrorState`: visible danger boundary, safe user-facing message, and retry path where possible.
- `ChartContainer`: title, provenance/coverage, plot, and legend form one unit.
- `Legend`: uses line/fill markers plus text; never color alone.

## Tables

Headers remain visible, labels align left, and numeric columns align right. UUIDs truncate visually but retain the full value in `title`. Missing values render as `—`; explanatory contexts may use `Unavailable` or `Not comparable`. A database `NULL` is never converted into numeric zero.

Rows use a 32–36 px rhythm. Hover changes only the surface. Selected rows use the accent border and remain keyboard reachable.

## Charts

Phase 4.0 supports only SVG bar, stacked-bar, line, histogram, and percentile/distribution views. Axes and grid lines use border tokens. Provider-reported and locally-estimated series use different line patterns as well as color. Every estimated composition plot displays estimator coverage adjacent to its title. Absent samples create gaps; they are not plotted as zero.

No pie/donut chart is used for precise category comparison. Animations are optional and disabled when reduced motion is requested.

## Interaction states

- Hover: move one surface level up without changing geometry.
- Focus: visible 2 px accent outline with 2 px offset.
- Disabled: reduced contrast, `not-allowed`, and an explicit reason such as “Coming soon.”
- Loading: skeleton or textual loading state occupies the expected content area.
- Empty: distinguish “no records” from “filter matched no records.”
- Error: safe typed API message; never raw SQL, paths, or stack traces.

## Measurement language

Provider usage carries `Provider reported`; context estimates carry `Locally estimated`; missing values carry `Unavailable`. Repetition is presented beside cache ratio with “Correlation does not imply provider cache equivalence.” Unknown content is descriptive and is never automatically recommended for compression. Opportunity ranking uses “Candidate priority,” never “potential savings.”

## Accessibility

Use landmark elements, semantic headings/tables, keyboard-reachable links and controls, visible focus, `aria-label` on icon actions, and contrast suitable for the dark palette. Tooltips must also be reachable by focus. This is a practical accessibility baseline rather than a Phase 4.0 certification claim.
