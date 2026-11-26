import { writable } from 'svelte/store';

// Split-pane terminal layout state. Supports single view (`none`), side-by-side
// (`horizontal`: left/right), and stacked (`vertical`: top/bottom) with two live
// terminal sessions rendered concurrently.

export type SplitDirection = 'none' | 'horizontal' | 'vertical';
export type PaneSlot = 'primary' | 'secondary';

export interface SplitLayout {
  direction: SplitDirection;
  secondarySessionId: number | null;
  focusedPane: PaneSlot;
}

const INITIAL: SplitLayout = {
  direction: 'none',
  secondarySessionId: null,
  focusedPane: 'primary'
};

function createSplitLayout() {
  const { subscribe, set, update } = writable<SplitLayout>({ ...INITIAL });

  return {
    subscribe,
    split(direction: Exclude<SplitDirection, 'none'>, secondarySessionId: number): void {
      set({ direction, secondarySessionId, focusedPane: 'secondary' });
    },
    setDirection(direction: Exclude<SplitDirection, 'none'>): void {
      update((cur) => (cur.secondarySessionId != null ? { ...cur, direction } : cur));
    },
    focus(pane: PaneSlot): void {
      update((cur) => (cur.direction === 'none' ? cur : { ...cur, focusedPane: pane }));
    },
    unsplit(): void {
      set({ ...INITIAL });
    },
    /** Clean up split state when a session tab closes or becomes the primary tab. */
    onSessionClosed(id: number): void {
      update((cur) => (cur.secondarySessionId === id ? { ...INITIAL } : cur));
    },
    /** Prevent showing the exact same session in both panes simultaneously. */
    ensureDistinct(primaryId: number | null): void {
      update((cur) =>
        cur.secondarySessionId != null && cur.secondarySessionId === primaryId ? { ...INITIAL } : cur
      );
    }
  };
}

export const splitLayout = createSplitLayout();
