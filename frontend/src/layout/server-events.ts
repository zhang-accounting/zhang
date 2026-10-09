import { type ReloadFailure } from './reload-failure.ts';

/** The events `/api/sse` sends, as the shell reads them. */
export type ServerEvent =
  { type: 'Reload' } | ({ type: 'ReloadFailed' } & ReloadFailure) | { type: 'Connected' } | { type: 'NewVersionFound'; version: string };

/** What the shell does on each event; `useServerEvents` binds these to toasts, atoms and the auth status. */
export interface ServerEventActions {
  /** The ledger may have changed: the server reloaded it, or the connection was down and its reloads were never delivered. */
  ledgerChanged(): void;
  reloaded(): void;
  reloadFailed(failure: ReloadFailure): void;
  /** `afterOutage`: this connection ends an offline period, rather than being the one of the page load. */
  connected(afterOutage: boolean): void;
  newVersion(version: string): void;
  /** `closed`: the server refused the stream for good (an error status such as 401), which EventSource does not retry. */
  disconnected(closed: boolean): void;
}

/**
 * Handles the stream's messages and errors across outages. "Connected" arrives on every page load and again after each
 * reconnect. A `Reload` broadcast while the connection was down is lost, so a reconnect counts as a ledger change: every
 * open page refreshes once, as it would have on the missed reload.
 */
export function serverEventHandler(actions: ServerEventActions) {
  let wasOffline = false;
  return {
    message(event: ServerEvent | undefined) {
      switch (event?.type) {
        case 'Reload':
          actions.reloaded();
          actions.ledgerChanged();
          break;
        case 'ReloadFailed':
          actions.reloadFailed(event);
          break;
        case 'Connected': {
          const afterOutage = wasOffline;
          wasOffline = false;
          actions.connected(afterOutage);
          if (afterOutage) actions.ledgerChanged();
          break;
        }
        case 'NewVersionFound':
          actions.newVersion(event.version);
          break;
        default:
          break;
      }
    },
    error(closed: boolean) {
      wasOffline = true;
      actions.disconnected(closed);
    },
  };
}
