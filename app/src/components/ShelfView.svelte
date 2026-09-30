<script lang="ts">
  import { appState } from "../stores/app-state.svelte";
  import { SECTION_TITLES } from "../lib/chrome";
  import { firstStoryDawLabel } from "../lib/card";
  import ShelfCard from "./ShelfCard.svelte";
</script>

<section class="stack">
  <h1>{SECTION_TITLES.shelf}</h1>
  <div class="shelf-grid">
    {#each appState.library?.shelf ?? [] as card (card.song_id)}
      <ShelfCard
        {card}
        dawLabel={appState.library ? firstStoryDawLabel(appState.library, card) : null}
        onSelect={() => appState.selectSong(card.song_id)}
      />
    {/each}
  </div>
</section>

<style>
  h1 {
    font-size: 18px;
    font-weight: 500;
    margin: 0;
  }
  .shelf-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: 12px;
  }
</style>
