<script lang="ts">
  import type { SongHeader } from "../lib/story";

  let { header }: { header: SongHeader } = $props();
</script>

<div class="row-wrap" style="justify-content: space-between; align-items: flex-start;">
  <div>
    <div class="title name-span" dir="auto">
      {header.title}{#if header.lineage}<span class="secondary"> · {header.lineage}</span>{/if}
    </div>
    <div class="subtitle secondary">{header.subtitle}</div>
  </div>
  <div class="row-wrap">
    <span class="pill">{header.kept.label}</span>
    {#if header.family_label}
      <span class="pill">{header.family_label}</span>
    {/if}
  </div>
</div>

<style>
  .title {
    font-size: 18px;
    font-weight: 500;
  }
  .subtitle {
    font-size: 13px;
  }
</style>
