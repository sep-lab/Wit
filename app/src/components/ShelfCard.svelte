<script lang="ts">
  import type { ShelfCard as ShelfCardT } from "../lib/story";
  import Artwork from "./Artwork.svelte";

  let { card, onSelect }: { card: ShelfCardT; onSelect: () => void } = $props();
</script>

<button type="button" class="card shelf-card" onclick={onSelect}>
  <Artwork artwork={card.artwork} />
  <div class="shelf-card-title name-span" dir="auto">{card.title}</div>
  <!-- DAW as Story text (review round 1, blocking #2) — two songs of the
       same title in different DAWs must not look identical. -->
  <div class="muted">{card.daw_label}</div>
  <div class="row-wrap">
    <!-- The moments pill, credited to whoever kept them
         (ShelfCard.moments_label — the wit-story contract PR this lane's
         review round 1 was waiting on). Never a UI-invented "N moments
         kept": every fixture story has kept_by_wit 0, and wit-story's own
         wording already distinguishes "kept" from "on disk". -->
    <span class="pill">{card.moments_label}</span>
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
