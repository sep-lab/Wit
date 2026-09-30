<script lang="ts">
  import type { Artwork as ArtworkT } from "../lib/story";

  let { artwork }: { artwork: ArtworkT } = $props();

  // A song always gets the same picture (Story contract: Artwork::Generated
  // seed). Purely decorative — the seed picks a hue, nothing more.
  function hue(seed: number): number {
    return seed % 360;
  }
</script>

{#if artwork.kind === "window_image"}
  <!--
    Logic's own WindowImage.jpg — the IPC command that fetches it by
    SongId doesn't exist yet (this lane's brief, item 4). Honest
    placeholder, not a fake picture.
  -->
  <div class="artwork artwork-placeholder">
    <span class="muted">Artwork not available yet</span>
  </div>
{:else if artwork.kind === "generated"}
  <div
    class="artwork artwork-generated"
    role="img"
    aria-label="Generated artwork"
    style={`background: linear-gradient(135deg, hsl(${hue(artwork.seed)} 55% 55%), hsl(${(hue(artwork.seed) + 40) % 360} 55% 32%));`}
  ></div>
{:else}
  <!-- An Artwork kind this build doesn't know about yet (wit-story
       lib.rs's compatibility rule: an unknown variant renders as text,
       skips styling — review round 1, non-blocking #10). The `{:else}`
       branch used to assume "generated" for anything that wasn't
       "window_image", which read `artwork.seed` as `undefined` and drew
       `hsl(NaN 55% 55%, ...)`. -->
  <div class="artwork artwork-placeholder">
    <span class="muted">Artwork not available yet</span>
  </div>
{/if}

<style>
  .artwork {
    width: 100%;
    aspect-ratio: 1;
    border-radius: calc(var(--radius) - 2px);
  }
  .artwork-placeholder {
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--surface-1);
    border: 0.5px dashed var(--border-strong);
    text-align: center;
    padding: 8px;
  }
</style>
