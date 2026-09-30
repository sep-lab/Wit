import type { Span, SpanKind } from "./story";

export interface RenderedSpan {
  /** Verbatim `span.text` — never parsed, never trusted as markup. */
  text: string;
  className: string;
  /** Set `dir="auto"` and `unicode-bidi: isolate` on this span: it may
   * hold a name the musician chose in Persian or any other RTL script
   * (this lane's brief, "Laws"). */
  rtl: boolean;
}

/** Bold: a track or plugin name stands alone in the sentence (see
 * `wit-story`'s `sentence.rs`: only these two kinds aren't already quoted
 * in the plain text). Every other name kind renders at normal weight — the
 * surrounding quotes are already part of `span.text`. */
const BOLD_KINDS: ReadonlySet<SpanKind> = new Set(["track", "plugin"]);

/** Kinds that hold a name the musician chose, as opposed to Wit's own
 * words (`plain`) or a rendered number (`value`). */
const NAME_KINDS: ReadonlySet<SpanKind> = new Set([
  "track",
  "region",
  "file",
  "plugin",
  "marker",
  "name",
]);

/**
 * One `Sentence.spans` entry → what a component needs to render it as
 * *text*: a CSS class chosen from `span.kind`, never a transformation of
 * `span.text` itself. This lane's brief, "Laws": "Names are data, never
 * markup. Render every span as text (no `{@html}` anywhere); style by
 * `span.kind`."
 */
export function renderSpan(span: Span): RenderedSpan {
  const rtl = NAME_KINDS.has(span.kind);
  const classes = [`span-${span.kind}`];
  if (BOLD_KINDS.has(span.kind)) classes.push("span-bold");
  // `.name-span` (app.css) sets `unicode-bidi: isolate`; paired in the
  // component with `dir="auto"` on the same element (this lane's brief,
  // "Laws": names may be RTL).
  if (rtl) classes.push("name-span");
  return {
    text: span.text,
    className: classes.join(" "),
    rtl,
  };
}

export function renderSpans(spans: readonly Span[]): RenderedSpan[] {
  return spans.map(renderSpan);
}
