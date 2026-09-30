<script lang="ts">
  import type { ShelfCard as ShelfCardT } from "../lib/story";
  import Artwork from "./Artwork.svelte";

  let {
    card,
    dawLabel,
    onSelect,
  }: { card: ShelfCardT; dawLabel: string | null; onSelect: () => void } = $props();
</script>

<button type="button" class="card shelf-card" onclick={onSelect}>
  <Artwork artwork={card.artwork} />
  <div class="shelf-card-title name-span" dir="auto">{card.title}</div>
  <div class="row-wrap">
    <!-- DAW shown as Story text, never a UI enum mapping (review round 1,
         blocking #2) — two songs of the same name in different DAWs must
         not look identical. No moments-kept pill here: ShelfCard has no
         rendered label for that yet (moments_label is landing in a small
         wit-story contract PR) and moments_kept alone can't be worded
         honestly without re-deriving wit-story's own "kept" logic. -->
    {#if dawLabel}
      <span class="pill">{dawLabel}</span>
    {/if}
    {#if card.last_worked_label}
      <span class="pill">{card.last_worked_label}</span>
    {/if}
    {#if card.copies_label}
      <span class="pill">{card.copies_label}</span>
    {/if}
  </div>
  <div class="muted">{card.digest}</div>
</button>

<style>
  .shelf-card {
    display: flex;
    flex-direction: column;
    gap: 6px;
    text-align: left;
    align-items: stretch;
  }
  .shelf-card-title {
    font-size: 15px;
    font-weight: 500;
  }
</style>
