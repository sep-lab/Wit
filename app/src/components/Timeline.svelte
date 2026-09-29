<script lang="ts">
  import type { Moment, Session } from "../lib/story";

  let {
    sessions,
    selectedMomentId,
    compareRange,
    onSelectMoment,
    onSelectRange,
  }: {
    sessions: readonly Session[];
    selectedMomentId: string | null;
    compareRange: readonly [string, string] | null;
    onSelectMoment: (id: string) => void;
    onSelectRange: (from: string, to: string) => void;
  } = $props();

  const flat = $derived(sessions.flatMap((s) => s.moments));
  const maxWeight = $derived(Math.max(1, ...flat.map((m) => m.weight)));

  function tickHeight(m: Moment): number {
    if (m.weight === 0) return 8;
    return 8 + Math.round((m.weight / maxWeight) * 36);
  }

  // Grey, folded routine saves (this lane's brief, "Laws").
  function isFolded(m: Moment): boolean {
    return m.verdict === "nothing_visible" || m.verdict === "identical";
  }

  function isSelected(m: Moment): boolean {
    if (compareRange) return compareRange[0] === m.id || compareRange[1] === m.id;
    return selectedMomentId === m.id;
  }

  // Drag across the timeline to pick two moments (this lane's brief, item
  // 4); a plain click (down and up on the same tick) selects one moment.
  let dragStart: string | null = $state(null);
  let dragging = $state(false);

  function pointerDown(id: string) {
    dragStart = id;
    dragging = false;
  }
  function pointerEnter(id: string) {
    if (dragStart !== null && dragStart !== id) dragging = true;
  }
  function pointerUp(id: string) {
    if (dragging && dragStart !== null && dragStart !== id) {
      const startIdx = flat.findIndex((m) => m.id === dragStart);
      const endIdx = flat.findIndex((m) => m.id === id);
      const [fromId, toId] = startIdx <= endIdx ? [dragStart, id] : [id, dragStart];
      onSelectRange(fromId, toId);
    } else {
      onSelectMoment(id);
    }
    dragStart = null;
    dragging = false;
  }
</script>

<div class="scroll-x">
  <div class="sessions">
    {#each sessions as session (session.id)}
      <div class="session">
        <div class="ticks">
          {#each session.moments as moment (moment.id)}
            <button
              type="button"
              class="tick"
              class:folded={isFolded(moment)}
              class:sel={isSelected(moment)}
              style={`height:${tickHeight(moment)}px`}
              title={moment.label}
              aria-label={moment.label}
              aria-pressed={isSelected(moment)}
              onpointerdown={() => pointerDown(moment.id)}
              onpointerenter={() => pointerEnter(moment.id)}
              onpointerup={() => pointerUp(moment.id)}
            ></button>
          {/each}
        </div>
        <div class="session-label secondary">{session.label}</div>
      </div>
    {/each}
  </div>
</div>

<style>
  .sessions {
    display: flex;
    gap: 18px;
    align-items: flex-end;
    padding-bottom: 4px;
  }
  .session {
    display: flex;
    flex-direction: column;
  }
  .ticks {
    display: flex;
    gap: 5px;
    height: 44px;
    align-items: flex-end;
  }
  .tick {
    width: 14px;
    min-width: 14px;
    padding: 0;
    border-radius: 3px;
    background: var(--tick-color);
    border: none;
    align-self: flex-end;
    touch-action: none;
  }
  .tick.folded {
    background: var(--border-strong);
  }
  .tick.sel {
    outline: 2px solid var(--text-primary);
    outline-offset: 2px;
  }
  .session-label {
    font-size: 12px;
    margin-top: 6px;
    white-space: nowrap;
  }
</style>
