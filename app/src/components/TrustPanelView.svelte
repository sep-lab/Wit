<script lang="ts">
  import { appState } from "../stores/app-state.svelte";
  import { COUNTER_LABELS, EMPTY_STATES, NAV_LABELS, SECTION_TITLES } from "../lib/chrome";

  const trust = $derived(appState.library?.trust ?? null);

  let showPreview = $state(false);
  let copyStatus = $state<string | null>(null);

  function reportText(): string {
    if (!trust) return "";
    const c = trust.counters;
    // Counts only — no names, no paths (this lane's brief, item 4: the
    // Story contract's own rule for TrustPanel/PilotCounters).
    return [
      `Compares opened: ${c.compares_opened}`,
      `Shares created: ${c.shares_created}`,
      `Restores made: ${c.restores_made}`,
    ].join("\n");
  }

  function togglePreview() {
    showPreview = !showPreview;
    copyStatus = null;
  }

  async function confirmCopy() {
    try {
      await navigator.clipboard.writeText(reportText());
      copyStatus = EMPTY_STATES.reportCopied;
    } catch {
      copyStatus = EMPTY_STATES.reportCopyFailed;
    }
  }
</script>

<section class="stack">
  <h1>{NAV_LABELS.trust}</h1>

  {#if trust}
    <div class="card stack">
      <h2>{SECTION_TITLES.trustWatched}</h2>
      {#if trust.watched_folders.length > 0}
        <ul class="plain-list">
          {#each trust.watched_folders as folder}
            <li class="name-span" dir="auto">{folder}</li>
          {/each}
        </ul>
      {:else}
        <p class="muted">{EMPTY_STATES.noLibrary}</p>
      {/if}
    </div>

    <div class="card stack">
      <h2>{SECTION_TITLES.trustStatements}</h2>
      <ul class="plain-list">
        {#each trust.statements as statement}
          <li>{statement}</li>
        {/each}
      </ul>
    </div>

    <div class="card stack">
      <h2>{SECTION_TITLES.trustCounters}</h2>
      <div class="row-wrap">
        <span class="pill">{COUNTER_LABELS.compares_opened}: {trust.counters.compares_opened}</span>
        <span class="pill">{COUNTER_LABELS.shares_created}: {trust.counters.shares_created}</span>
        <span class="pill">{COUNTER_LABELS.restores_made}: {trust.counters.restores_made}</span>
      </div>

      <div class="stack">
        <button type="button" onclick={togglePreview}>{SECTION_TITLES.copyReport}</button>
        {#if showPreview}
          <div class="report-preview">
            <div class="muted">{SECTION_TITLES.copyReportPreviewTitle}</div>
            <pre>{reportText()}</pre>
            <button type="button" onclick={confirmCopy}>{SECTION_TITLES.confirmCopy}</button>
            {#if copyStatus}
              <span class="muted">{copyStatus}</span>
            {/if}
          </div>
        {/if}
      </div>
    </div>
  {/if}
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
  .plain-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .report-preview {
    border: 0.5px solid var(--border);
    border-radius: var(--radius);
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  pre {
    margin: 0;
    font-size: 12px;
    white-space: pre-wrap;
  }
</style>
