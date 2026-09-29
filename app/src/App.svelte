<script lang="ts">
  import { appState } from "./stores/app-state.svelte";
  import { EMPTY_STATES, NAV_LABELS } from "./lib/chrome";
  import ShelfView from "./components/ShelfView.svelte";
  import SongView from "./components/SongView.svelte";
  import FamilyView from "./components/FamilyView.svelte";
  import SendReadyView from "./components/SendReadyView.svelte";
  import TrustPanelView from "./components/TrustPanelView.svelte";

  appState.load();
</script>

<div class="page stack">
  <nav class="row-wrap" aria-label="Sections">
    <button
      type="button"
      class:active={appState.screen === "shelf"}
      onclick={() => appState.goTo("shelf")}
    >
      {NAV_LABELS.shelf}
    </button>
    <button
      type="button"
      class:active={appState.screen === "song"}
      disabled={!appState.currentStory}
      onclick={() => appState.goTo("song")}
    >
      {NAV_LABELS.song}
    </button>
    <button
      type="button"
      class:active={appState.screen === "family"}
      disabled={!appState.currentStory}
      onclick={() => appState.goTo("family")}
    >
      {NAV_LABELS.family}
    </button>
    <button
      type="button"
      class:active={appState.screen === "send-ready"}
      disabled={!appState.currentStory}
      onclick={() => appState.goTo("send-ready")}
    >
      {NAV_LABELS.send_ready}
    </button>
    <button
      type="button"
      class:active={appState.screen === "trust"}
      onclick={() => appState.goTo("trust")}
    >
      {NAV_LABELS.trust}
    </button>
  </nav>

  {#if appState.loading}
    <p class="muted">{EMPTY_STATES.loading}</p>
  {:else if appState.loadError}
    <p class="muted">{appState.loadError}</p>
  {:else if !appState.library || appState.library.shelf.length === 0}
    <p class="muted">{EMPTY_STATES.noLibrary}</p>
  {:else if appState.screen === "shelf"}
    <ShelfView />
  {:else if appState.screen === "song"}
    <SongView />
  {:else if appState.screen === "family"}
    <FamilyView />
  {:else if appState.screen === "send-ready"}
    <SendReadyView />
  {:else}
    <TrustPanelView />
  {/if}
</div>

<style>
  nav {
    border-bottom: 0.5px solid var(--border);
    padding-bottom: 8px;
  }
  nav button.active {
    background: var(--surface-1);
    border-color: var(--border-stronger);
  }
</style>
