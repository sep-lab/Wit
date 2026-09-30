<script lang="ts">
  import type { Comparison } from "../lib/story";
  import { appState } from "../stores/app-state.svelte";
  import { cardFromComparison, cardFromMoment, cardFromStubbedCompare, findMoment } from "../lib/card";
  import { EMPTY_STATES, SECTION_TITLES } from "../lib/chrome";
  import { userFacingErrorMessage } from "../lib/errors";
  import * as ipc from "../lib/ipc";
  import SongHeaderView from "./SongHeaderView.svelte";
  import Timeline from "./Timeline.svelte";
  import HeatStrip from "./HeatStrip.svelte";
  import ChangeCard from "./ChangeCard.svelte";

  let compareResult = $state<Comparison | null>(null);
  let compareError = $state<string | null>(null);
  let actionError = $state<string | null>(null);

  const story = $derived(appState.currentStory);
  const flatMoments = $derived(story ? story.sessions.flatMap((s) => s.moments) : []);
  // True on the default first view — no moment or range picked, so the
  // card is the Story's own oldest-to-newest overview (review round 1,
  // non-blocking #5: highlight this state and give a way back to it).
  const onOverview = $derived(!appState.selectedMomentId && !appState.compareRange);

  // Drag-compare (this lane's brief, item 4): actually call the engine's
  // `compare` command rather than guessing — today it always rejects with
  // NotAvailableError (it's a stub), so the change card shows that
  // honestly instead of invented sentences. The day `compare` stops being
  // a stub, this starts showing real results with no UI change.
  $effect(() => {
    const range = appState.compareRange;
    const s = story;
    compareResult = null;
    compareError = null;
    if (!range || !s) return;
    const [from, to] = range;
    let cancelled = false;
    ipc
      .compareMoments(s.id, from, to)
      .then((result) => {
        if (!cancelled) compareResult = result;
      })
      .catch((err) => {
        if (!cancelled) compareError = userFacingErrorMessage(err);
      });
    return () => {
      cancelled = true;
    };
  });

  const card = $derived.by(() => {
    if (!story) return null;
    if (appState.compareRange) {
      const [from, to] = appState.compareRange;
      if (compareResult) return cardFromComparison(compareResult, story);
      return cardFromStubbedCompare(
        story,
        from,
        to,
        compareError ?? EMPTY_STATES.compareUnavailable
      );
    }
    if (appState.selectedMomentId) {
      const m = findMoment(story, appState.selectedMomentId);
      if (m) return cardFromMoment(m);
    }
    // First view: the overview compare, oldest to newest — never an
    // adjacent pair (this lane's brief, item 4).
    if (story.overview) return cardFromComparison(story.overview, story);
    return flatMoments[0] ? cardFromMoment(flatMoments[0]) : null;
  });

  async function handleOpenAsCopy() {
    actionError = null;
    if (!story || !card?.momentId) return;
    try {
      await ipc.openAsCopy(story.id, card.momentId);
    } catch (err) {
      actionError = userFacingErrorMessage(err);
    }
  }

  async function handleSend() {
    actionError = null;
    if (!story || !card?.momentId) return;
    try {
      await ipc.sendToFriend(story.id, card.momentId);
    } catch (err) {
      actionError = userFacingErrorMessage(err);
    }
  }

  function handleListen() {
    // Disabled until there is source audio to play (Actions.listen); no
    // command is wired yet — see "Needs from other lanes" in the PR body.
  }
</script>

{#if story}
  <section class="stack">
    <div class="card">
      <SongHeaderView header={story.header} />

      <div style="margin-top: 1.25rem;">
        <Timeline
          sessions={story.sessions}
          selectedMomentId={appState.selectedMomentId}
          compareRange={appState.compareRange}
          onSelectMoment={(id) => appState.selectMoment(id)}
          onSelectRange={(from, to) => appState.selectCompareRange(from, to)}
        />
      </div>

      <div style="margin-top: 1rem;">
        <HeatStrip
          tracks={story.tracks}
          moments={flatMoments}
          selectedMomentId={appState.selectedMomentId}
          compareRange={appState.compareRange}
        />
      </div>

      {#if card}
        <div style="margin-top: 1.25rem;">
          <div class="row-wrap overview-row">
            {#if onOverview}
              <span class="pill">{SECTION_TITLES.overview}</span>
            {:else}
              <button type="button" onclick={() => appState.showOverview()}>
                {SECTION_TITLES.backToOverview}
              </button>
            {/if}
          </div>
          <ChangeCard
            {card}
            capability={story.capability}
            onListen={handleListen}
            onOpenAsCopy={handleOpenAsCopy}
            onSend={handleSend}
          />
          {#if actionError}
            <p class="muted">{actionError}</p>
          {/if}
        </div>
      {/if}
    </div>
  </section>
{:else}
  <p class="muted">{EMPTY_STATES.noSongSelected}</p>
{/if}

<style>
  .overview-row {
    margin-bottom: 6px;
  }
</style>
