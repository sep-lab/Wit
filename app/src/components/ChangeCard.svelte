<script lang="ts">
  import type { CapabilityNote } from "../lib/story";
  import type { CardData } from "../lib/card";
  import { ACTION_LABELS } from "../lib/chrome";
  import { disabledReason } from "../lib/actions";
  import SentenceLine from "./SentenceLine.svelte";

  let {
    card,
    capability,
    onListen,
    onOpenAsCopy,
    onSend,
  }: {
    card: CardData;
    capability: readonly CapabilityNote[];
    onListen: () => void;
    onOpenAsCopy: () => void;
    onSend: () => void;
  } = $props();

  const listenReason = $derived(disabledReason(card.actions, "listen"));
  const openAsCopyReason = $derived(disabledReason(card.actions, "open_as_copy"));
  const sendReason = $derived(disabledReason(card.actions, "send"));
</script>

<div class="change-card">
  <div class="row-wrap" style="justify-content: space-between; align-items: baseline;">
    <div class="card-heading">{card.heading}</div>
    <div class="sub secondary">{card.subheading}</div>
  </div>

  <div class="list">
    {#if card.stubNote}
      <p class="muted">{card.stubNote}</p>
    {:else if card.sentences.length > 0}
      {#if card.summary}
        <p class="summary">{card.summary}</p>
      {/if}
      {#each card.sentences as sentence}
        <SentenceLine {sentence} />
      {/each}
    {:else if card.note}
      <p class="muted">{card.note}</p>
    {/if}
  </div>

  <div class="row-wrap actions">
    <div class="action">
      <button type="button" disabled={!card.actions.listen} onclick={onListen}>
        {ACTION_LABELS.listen}
      </button>
      {#if listenReason}
        <div class="disabled-reason">{listenReason}</div>
      {/if}
    </div>
    <div class="action">
      <button type="button" disabled={!card.actions.open_as_copy} onclick={onOpenAsCopy}>
        {ACTION_LABELS.open_as_copy}
      </button>
      {#if openAsCopyReason}
        <div class="disabled-reason">{openAsCopyReason}</div>
      {/if}
    </div>
    <div class="action">
      <button type="button" disabled={!card.actions.send} onclick={onSend}>
        {ACTION_LABELS.send}
      </button>
      {#if sendReason}
        <div class="disabled-reason">{sendReason}</div>
      {/if}
    </div>
  </div>

  {#if capability.length > 0}
    <div class="capability muted">
      {#each capability as note}
        <div>{note.text}</div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .change-card {
    border: 0.5px solid var(--border);
    border-radius: 12px;
    padding: 0.75rem 1rem;
    background: var(--surface-1);
  }
  .card-heading {
    font-size: 15px;
    font-weight: 500;
  }
  .sub {
    font-size: 12px;
  }
  .summary {
    font-size: 13px;
    color: var(--text-secondary);
    margin: 4px 0;
  }
  .actions {
    margin-top: 10px;
  }
  .action {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .capability {
    margin-top: 10px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
</style>
