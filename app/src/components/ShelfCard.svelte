<script lang="ts">
  import type { ShelfCard as ShelfCardT } from "../lib/story";
  import Artwork from "./Artwork.svelte";

  let { card, onSelect }: { card: ShelfCardT; onSelect: () => void } = $props();
</script>

<button type="button" class="card shelf-card" onclick={onSelect}>
  <Artwork artwork={card.artwork} />
  <div class="shelf-card-title name-span" dir="auto">{card.title}</div>
  {#if card.last_worked_label}
    <div class="muted">{card.last_worked_label}</div>
  {/if}
  <div class="row-wrap">
    <span class="pill">{card.moments_kept} {card.moments_kept === 1 ? "moment" : "moments"} kept</span>
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
