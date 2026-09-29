<script lang="ts">
  import type { Screen } from "./stores/app-state.svelte";
  import { appState } from "./stores/app-state.svelte";
  import { EMPTY_STATES, NAV_LABELS } from "./lib/chrome";
  import { hasShortcutModifier, modifierKeyLabel } from "./lib/platform";
  import ShelfView from "./components/ShelfView.svelte";
  import SongView from "./components/SongView.svelte";
  import FamilyView from "./components/FamilyView.svelte";
  import SendReadyView from "./components/SendReadyView.svelte";
  import TrustPanelView from "./components/TrustPanelView.svelte";

  appState.load();

  // ⌘ on Mac, Ctrl elsewhere (this lane's brief, "Layout & theming").
  const SCREENS: Screen[] = ["shelf", "song", "family", "send-ready", "trust"];
  const modLabel = modifierKeyLabel();

  function shortcutHint(index: number): string {
    return `${modLabel}${index + 1}`;
  }

  function handleKeydown(event: KeyboardEvent) {
    if (!hasShortcutModifier(event)) return;
    const index = SCREENS.findIndex((_, i) => event.key === String(i + 1));
    if (index === -1) return;
    const screen = SCREENS[index];
    if (screen === "shelf" || screen === "trust" || appState.currentStory) {
      event.preventDefault();
      appState.goTo(screen);
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="page stack">
  <nav class="row-wrap" aria-label="Sections">
    <button
      type="button"
      class:active={appState.screen === "shelf"}
      title={shortcutHint(0)}
      aria-keyshortcuts={shortcutHint(0)}
      onclick={() => appState.goTo("shelf")}
    >
      {NAV_LABELS.shelf}
    </button>
    <button
      type="button"
      class:active={appState.screen === "song"}
      disabled={!appState.currentStory}
      title={shortcutHint(1)}
      aria-keyshortcuts={shortcutHint(1)}
      onclick={() => appState.goTo("song")}
    >
      {NAV_LABELS.song}
    </button>
    <button
      type="button"
      class:active={appState.screen === "family"}
      disabled={!appState.currentStory}
      title={shortcutHint(2)}
      aria-keyshortcuts={shortcutHint(2)}
      onclick={() => appState.goTo("family")}
    >
      {NAV_LABELS.family}
    </button>
    <button
      type="button"
      class:active={appState.screen === "send-ready"}
      disabled={!appState.currentStory}
      title={shortcutHint(3)}
      aria-keyshortcuts={shortcutHint(3)}
      onclick={() => appState.goTo("send-ready")}
    >
      {NAV_LABELS.send_ready}
    </button>
    <button
      type="button"
      class:active={appState.screen === "trust"}
      title={shortcutHint(4)}
      aria-keyshortcuts={shortcutHint(4)}
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
