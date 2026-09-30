<script lang="ts">
  import type { FamilyMember } from "../lib/story";
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

  // A relation kind this build doesn't recognize yet renders its raw text
  // rather than an empty pill (wit-story's own compatibility rule: an
  // unknown variant renders as text, skips styling — review round 1,
  // non-blocking #10).
  function relationLabel(kind: FamilyMember["relation"]["kind"]): string {
    return (RELATION_LABELS as Record<string, string>)[kind] ?? kind;
  }
</script>

{#snippet memberContent(member: FamilyMember)}
  <div class="row-wrap" style="justify-content: space-between;">
    <div class="name-span" dir="auto">{member.title}</div>
    {#if member.is_current}
      <span class="pill">{FAMILY_CURRENT_TAG}</span>
    {/if}
  </div>
  <div class="row-wrap">
    <span class="pill">{relationLabel(member.relation.kind)}</span>
    {#if member.last_worked}
      <span class="muted">{formatTimestamp(member.last_worked)}</span>
    {/if}
  </div>
{/snippet}

<section class="stack">
  <h1>{SECTION_TITLES.family}</h1>
  {#if family && family.members.length > 0}
    <ul class="family-list">
      {#each family.members as member, i (member.story_id ?? `${member.song_id}-${i}`)}
        <li>
          <!-- Every other member of the family is clickable — this used
               to have no way to reach a second lineage at all (review
               round 1, non-blocking #3). -->
          {#if member.story_id && !member.is_current}
            <button
              type="button"
              class="card family-member"
              onclick={() =>
                appState.selectFamilyMember(member.song_id, member.story_id)}
            >
              {@render memberContent(member)}
            </button>
          {:else}
            <div class="card family-member">
              {@render memberContent(member)}
            </div>
          {/if}
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
    width: 100%;
    text-align: left;
  }
</style>
