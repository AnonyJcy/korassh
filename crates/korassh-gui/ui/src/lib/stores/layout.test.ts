import { describe, expect, it } from 'vitest';
import { get } from 'svelte/store';
import { splitLayout } from './layout';

describe('splitLayout store', () => {
  it('splits horizontally/vertically, switches focus, and cleans up on close', () => {
    splitLayout.unsplit();
    expect(get(splitLayout)).toEqual({
      direction: 'none',
      secondarySessionId: null,
      focusedPane: 'primary'
    });

    splitLayout.split('horizontal', 42);
    expect(get(splitLayout)).toEqual({
      direction: 'horizontal',
      secondarySessionId: 42,
      focusedPane: 'secondary'
    });

    splitLayout.setDirection('vertical');
    splitLayout.focus('primary');
    expect(get(splitLayout)).toEqual({
      direction: 'vertical',
      secondarySessionId: 42,
      focusedPane: 'primary'
    });

    // Closing an unrelated session preserves the split.
    splitLayout.onSessionClosed(99);
    expect(get(splitLayout).secondarySessionId).toBe(42);

    // Selecting the secondary session as primary automatically unsplits.
    splitLayout.ensureDistinct(42);
    expect(get(splitLayout).direction).toBe('none');
  });
});
