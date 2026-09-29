import type { Library, Story } from "../lib/story";
import { fetchLibrary } from "../lib/ipc";

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
  screen = $state<Screen>("shelf");

  selectedSongId = $state<string | null>(null);
  selectedStoryId = $state<string | null>(null);
  /** A single selected moment (click), oldest-to-newest pair (drag), or
   * neither (falls back to the Story's own `overview`). */
  selectedMomentId = $state<string | null>(null);
  compareRange = $state<[string, string] | null>(null);

  async load(root: string | null = null): Promise<void> {
    this.loading = true;
    this.loadError = null;
    try {
      const library = await fetchLibrary(root);
      this.library = library;
      if (!this.selectedSongId && library.shelf.length > 0) {
        this.selectSong(library.shelf[0].song_id);
      }
    } catch (err) {
      this.loadError = err instanceof Error ? err.message : String(err);
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

  selectSong(songId: string): void {
    this.selectedSongId = songId;
    const stories = this.storiesForSong(songId);
    this.selectedStoryId = stories[0]?.id ?? null;
    this.selectedMomentId = null;
    this.compareRange = null;
    this.screen = "song";
  }

  selectStory(storyId: string): void {
    this.selectedStoryId = storyId;
    this.selectedMomentId = null;
    this.compareRange = null;
  }

  selectMoment(momentId: string): void {
    this.selectedMomentId = momentId;
    this.compareRange = null;
  }

  selectCompareRange(fromId: string, toId: string): void {
    this.compareRange = [fromId, toId];
    this.selectedMomentId = null;
  }

  goTo(screen: Screen): void {
    this.screen = screen;
  }
}

export const appState = new AppState();
