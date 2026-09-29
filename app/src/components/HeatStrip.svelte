<script lang="ts">
  import type { Moment, TrackLane } from "../lib/story";
  import { buildHeatMatrix, heatVar } from "../lib/heat";

  let {
    tracks,
    moments,
    selectedMomentId,
    compareRange,
  }: {
    tracks: readonly TrackLane[];
    moments: readonly Moment[];
    selectedMomentId: string | null;
    compareRange: readonly [string, string] | null;
  } = $props();

  const matrix = $derived(buildHeatMatrix(tracks.length, moments));

  function highlighted(id: string): boolean {
    if (compareRange) return compareRange[0] === id || compareRange[1] === id;
    return selectedMomentId === id;
  }
</script>

<!--
  Empty tracks -> no strip at all (this lane's brief, "Laws": "empty
  tracks → no strip, show the capability note instead"). The Story's own
  capability notes (already rendered under the change card) explain why —
  nothing extra is authored here.
-->
{#if tracks.length > 0}
  <div class="scroll-x">
    <div class="heat-strip">
      {#each tracks as track, ti (track.key)}
        <div class="row">
          <div class="lab name-span" dir="auto" title={track.name}>{track.name}</div>
          {#each moments as moment, mi (moment.id)}
            <div
              class="cell"
              class:sel={highlighted(moment.id)}
              style={`background:${heatVar(matrix[ti]?.[mi] ?? 0)}`}
            ></div>
          {/each}
        </div>
      {/each}
    </div>
  </div>
  <div class="muted">Each square is one save on one track. Darker means more changed.</div>
{/if}

<style>
  .heat-strip {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .row {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .lab {
    font-size: 12px;
    color: var(--text-secondary);
    width: 96px;
    flex: none;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .cell {
    width: 14px;
    height: 10px;
    border-radius: 2px;
    flex: none;
  }
  .cell.sel {
    outline: 1px solid var(--border-stronger);
  }
</style>
