// The app's update state (system/05, D0027): one instance shared by the banner and the Connect view.
import { checkUpdate, errorText, installUpdate, type UpdateStatus } from './api';

/** How often the page asks again while it stays open. The app answers from memory inside its own interval. */
export const UPDATE_POLL_MS = 6 * 60 * 60 * 1000 + 60_000;

export class UpdateState {
  status = $state<UpdateStatus | null>(null);
  checking = $state(false);
  installing = $state(false);
  /** Shown to the owner: a failed install, or a failed check they asked for. */
  error = $state<string | null>(null);

  /** `force` is the owner asking: it skips the app's memory and reports a failure. */
  async check(force = false): Promise<void> {
    if (this.checking) return;
    this.checking = true;
    if (force) this.error = null;
    try {
      this.status = await checkUpdate(force);
      if (force && this.status.error) this.error = this.status.error;
    } catch (e) {
      // Not in the desktop app (or the command failed): a background check stays silent.
      if (force) this.error = errorText(e);
    } finally {
      this.checking = false;
    }
  }

  /** On success the app restarts and this never returns. */
  async install(): Promise<void> {
    if (this.installing) return;
    this.installing = true;
    this.error = null;
    try {
      await installUpdate();
    } catch (e) {
      this.error = errorText(e);
      this.installing = false;
    }
  }
}
