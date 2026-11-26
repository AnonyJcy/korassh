import { get } from 'svelte/store';
import { activeEntity } from './activeEntity';
import { sessions, type Session, type SessionKind } from './sessions';
import { splitLayout, type SplitDirection } from './layout';

// Composed navigation actions that keep the sessions list and the active entity in
// step (tech-gui.md §2). A spawn appends a session and makes it active (both spawn
// paths do this); closing the active session falls back to the Dashboard so Content
// is never left pointing at a closed tab.
export function spawnSession(kind: SessionKind, hostName: string): Session {
  const session = sessions.spawn(kind, hostName);
  splitLayout.ensureDistinct(session.id);
  activeEntity.activateSession(session.id);
  return session;
}

/** Open a local shell terminal tab (PowerShell / Bash / Zsh). */
export function spawnLocalSession(label = 'Local Shell'): Session {
  const session = sessions.spawn('terminal', label, { isLocal: true });
  splitLayout.ensureDistinct(session.id);
  activeEntity.activateSession(session.id);
  return session;
}

/** Duplicate an existing session tab (SSH terminal, local shell, or SFTP). */
export function duplicateSession(source: Session): Session {
  return source.isLocal ? spawnLocalSession(source.hostName) : spawnSession(source.kind, source.hostName);
}

/** Open a duplicate/companion terminal in a split pane beside the current terminal. */
export function splitWithDuplicate(
  source: Session,
  direction: Exclude<SplitDirection, 'none'>
): Session {
  const created = sessions.spawn(
    'terminal',
    source.hostName,
    source.isLocal ? { isLocal: true } : undefined
  );
  splitLayout.split(direction, created.id);
  return created;
}

export function closeSession(id: number): void {
  const active = get(activeEntity);
  const split = get(splitLayout);
  if (active.kind === 'session' && active.id === id) {
    if (split.direction !== 'none' && split.secondarySessionId != null && split.secondarySessionId !== id) {
      const promote = split.secondarySessionId;
      splitLayout.unsplit();
      activeEntity.activateSession(promote);
    } else {
      splitLayout.unsplit();
      activeEntity.selectDashboard();
    }
  } else {
    splitLayout.onSessionClosed(id);
  }
  sessions.close(id);
}
