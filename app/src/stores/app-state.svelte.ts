import type { Library, Story } from "../lib/story";
import { fetchLibrary } from "../lib/ipc";
import { userFacingErrorMessage } from "../lib/errors";

export type Screen = "shelf" | "song" | "family" | "send-ready" | "trust";

/**
 * The app's shared reactive state, Svelte 5 runes in a class (the
 * documented pattern for cross-component state outside a single
 * component's own `<script>`). One instance, imported wherever it's
 * needed; every field is `$state`, so reading it in a template or a
 * `$derived` tracks it like a component's own state would.
 */
class AppState {
  library = $state<Library | null>(null);
  loadError = $state<string | null>(null);
  loading = $state(true);
  // The app opens on the Shelf, never the Song view (review round 1,
  // blocking #3) — preparing a default story selection below must not
  // change this.
  screen = $state<Screen>("shelf");

  selectedSongId = $state<string | null>(null);
  selectedStoryId = $state<string | null>(null);
  /** A single selected moment (click), oldest-to-newest pair (drag), or
   * neither (falls back to the Story's own `overview`). */
  selectedMomentId = $state<string | null>(null);
  compareRange = $state<[string, string] | null>(null);

  async load(): Promise<void> {
    this.loading = true;
    this.loadError = null;
    try {
      const library = await fetchLibrary();
      this.library = library;
      if (!this.selectedSongId && library.shelf.length > 0) {
        this.prepareSong(library.shelf[0].song_id);
      }
    } catch (err) {
      this.loadError = userFacingErrorMessage(err);
    } finally {
      this.loading = false;
    }
  }

  storiesForSong(songId: string): Story[] {
    return this.library ? this.library.stories.filter((s) => s.song_id === songId) : [];
  }

  get currentStory(): Story | null {
    if (!this.library || !this.selectedStoryId) return null;
    return this.library.stories.find((s) => s.id === this.selectedStoryId) ?? null;
  }

  /**
   * Select a song's default story without changing `screen` — the Shelf
   * card's own `story_ids[0]` (newest-worked first), not just the first
   * match `library.stories` happens to list (review round 1, non-blocking
   * #3: those two orders can disagree).
   */
  private prepareSong(songId: string): void {
    this.selectedSongId = songId;
    const card = this.library?.shelf.find((c) => c.song_id === songId);
    const preferredStoryId = card?.story_ids[0];
    const stories = this.storiesForSong(songId);
    this.selectedStoryId = preferredStoryId ?? stories[0]?.id ?? null;
    this.selectedMomentId = null;
    this.compareRange = null;
  }

  selectSong(songId: string): void {
    this.prepareSong(songId);
    this.screen = "song";
  }

  selectStory(storyId: string): void {
    this.selectedStoryId = storyId;
    this.selectedMomentId = null;
    this.compareRange = null;
  }

  /** A Family member may belong to a different song than the one being
   * viewed (a Save-As copy, a restore) — keep `selectedSongId` in sync
   * too, not just the story (review round 1, non-blocking #3: "Second
   * lineage unreachable" — Family members are otherwise unclickable). */
  selectFamilyMember(songId: string, storyId: string | null | undefined): void {
    if (!storyId) return;
    this.selectedSongId = songId;
    this.selectStory(storyId);
    this.screen = "song";
  }

  selectMoment(momentId: string): void {
    this.selectedMomentId = momentId;
    this.compareRange = null;
  }

  selectCompareRange(fromId: string, toId: string): void {
    this.compareRange = [fromId, toId];
    this.selectedMomentId = null;
  }

  /** Back to the Story's own overview compare (oldest → newest) — the
   * default first view (review round 1, non-blocking #5: give a way back
   * to it after clicking a tick). */
  showOverview(): void {
    this.selectedMomentId = null;
    this.compareRange = null;
  }

  goTo(screen: Screen): void {
    this.screen = screen;
  }
}

export const appState = new AppState();
