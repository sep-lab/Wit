<script lang="ts">
  import { appState } from "../stores/app-state.svelte";
  import { EMPTY_STATES, FAMILY_CURRENT_TAG, RELATION_LABELS, SECTION_TITLES } from "../lib/chrome";

  const story = $derived(appState.currentStory);
  const family = $derived(story?.family ?? null);

  // FamilyMember carries a raw Timestamp with no rendered label field
  // (unlike ShelfCard/Moment, which the engine gives a pre-rendered
  // string alongside every timestamp — see this lane's PR body, "Needs
  // from other lanes"). This uses the platform's own date formatting —
  // not invented copy, just a numeric-to-calendar conversion — rather
  // than re-implement wit-story's "Today/Yesterday/Thu" wording, which
  // that crate deliberately keeps to itself.
  function formatTimestamp(seconds: number): string {
    return new Date(seconds * 1000).toLocaleDateString(undefined, {
      year: "numeric",
      month: "short",
      day: "numeric",
    });
  }
</script>

<section class="stack">
  <h1>{SECTION_TITLES.family}</h1>
  {#if family && family.members.length > 0}
    <ul class="family-list">
      {#each family.members as member (member.story_id ?? member.song_id)}
        <li class="card family-member">
          <div class="row-wrap" style="justify-content: space-between;">
            <div class="name-span" dir="auto">{member.title}</div>
            {#if member.is_current}
              <span class="pill">{FAMILY_CURRENT_TAG}</span>
            {/if}
          </div>
          <div class="row-wrap">
            <span class="pill">{RELATION_LABELS[member.relation.kind]}</span>
            {#if member.last_worked}
              <span class="muted">{formatTimestamp(member.last_worked)}</span>
            {/if}
          </div>
        </li>
      {/each}
    </ul>
  {:else}
    <p class="muted">{EMPTY_STATES.noFamily}</p>
  {/if}
</section>

<style>
  h1 {
    font-size: 18px;
    font-weight: 500;
    margin: 0;
  }
  .family-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .family-member {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
</style>
