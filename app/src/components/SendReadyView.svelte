<script lang="ts">
  import { appState } from "../stores/app-state.svelte";
  import { EMPTY_STATES, SECTION_TITLES } from "../lib/chrome";

  const story = $derived(appState.currentStory);
  const sendReady = $derived(story?.send_ready ?? null);
</script>

<section class="stack">
  <h1>{SECTION_TITLES.sendReady}</h1>

  {#if sendReady}
    <div class="card stack">
      <div class="headline">{sendReady.headline}</div>
      {#if sendReady.plugins.length > 0}
        <ul class="plain-list">
          {#each sendReady.plugins as plugin}
            <li>
              <span class="name-span" dir="auto">{plugin.name}</span>
              {#if plugin.tracks.length > 0}
                <span class="muted">
                  — {plugin.tracks.join(", ")}
                </span>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
      {#if sendReady.outside_files.length > 0}
        <ul class="plain-list">
          {#each sendReady.outside_files as file}
            <li>
              <span class="name-span" dir="auto">{file.file_name}</span>
              <span class="muted">— {file.reason}</span>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  {:else}
    <p class="muted">{EMPTY_STATES.sendReadyUnknown}</p>
  {/if}

  <div class="stack">
    <h2>{SECTION_TITLES.bounceDropZone}</h2>
    <div class="drop-zone" aria-disabled="true">
      <span class="muted">{EMPTY_STATES.dropZoneDisabled}</span>
    </div>
  </div>
</section>

<style>
  h1 {
    font-size: 18px;
    font-weight: 500;
    margin: 0;
  }
  h2 {
    font-size: 14px;
    font-weight: 500;
    margin: 0;
  }
  .headline {
    font-size: 15px;
    font-weight: 500;
  }
  .plain-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .drop-zone {
    border: 1.5px dashed var(--border-strong);
    border-radius: var(--radius);
    padding: 24px;
    text-align: center;
    opacity: 0.6;
  }
</style>
