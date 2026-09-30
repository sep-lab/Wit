<script lang="ts">
  import type { Sentence } from "../lib/story";
  import { renderSpans } from "../lib/spans";
  import { iconGlyph } from "../lib/icons";

  let { sentence }: { sentence: Sentence } = $props();
  const glyph = $derived(iconGlyph(sentence.icon));
</script>

<div class="chg">
  <span class="ico" aria-hidden="true" title={glyph.label}>{glyph.glyph}</span>
  <div class="chg-text">
    <!-- Every span renders as text ({span.text}), never {@html} — a name
         the musician chose can be anything, and is never markup (this
         lane's brief, "Laws"). -->
    {#each renderSpans(sentence.spans) as span}
      <span class={span.className} dir={span.rtl ? "auto" : undefined}>{span.text}</span>
    {/each}
  </div>
  {#if sentence.place_label}
    <div class="chg-place secondary">{sentence.place_label}</div>
  {/if}
</div>

<style>
  .chg {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    padding: 8px 0;
    border-top: 0.5px solid var(--border);
    font-size: 14px;
  }
  .ico {
    font-size: 14px;
    color: var(--text-secondary);
    width: 18px;
    flex: none;
    text-align: center;
  }
  .chg-text {
    flex: 1;
  }
  .chg-place {
    font-size: 12px;
    white-space: nowrap;
  }
</style>
