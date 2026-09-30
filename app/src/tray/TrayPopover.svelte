<script lang="ts">
  import { appState } from "../stores/app-state.svelte";
  import { showMainWindow } from "../lib/ipc";
  import { EMPTY_STATES } from "../lib/chrome";

  appState.load();

  async function openMain() {
    try {
      await showMainWindow();
    } catch {
      // A plain-browser preview of the popover (no Tauri window to open).
    }
  }
</script>

<div class="tray">
  {#if appState.loading}
    <p class="muted">{EMPTY_STATES.loading}</p>
  {:else if appState.library && appState.library.shelf.length > 0}
    <!-- One line per song from `digest`. No badges, no notifications
         (this lane's brief, "Laws" and item 4 "Tray"). -->
    <ul class="tray-list">
      {#each appState.library.shelf as card (card.song_id)}
        <li>
          <span class="song-title">
            <span class="name-span" dir="auto">{card.title}</span>
            <span class="muted">· {card.daw_label}</span>
          </span>
          <span class="muted">{card.digest}</span>
        </li>
      {/each}
    </ul>
  {:else}
    <p class="muted">{EMPTY_STATES.noLibrary}</p>
  {/if}
  <button type="button" onclick={openMain}>Open Wit</button>
</div>

<style>
  :global(body) {
    background: var(--surface-0);
  }
  .tray {
    padding: 8px;
    font-size: 12px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .tray-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .song-title {
    display: block;
    font-weight: 500;
  }
</style>
