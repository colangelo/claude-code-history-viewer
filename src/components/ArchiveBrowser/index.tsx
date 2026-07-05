/**
 * Archive browser: browse and search the cross-machine hub archive
 * (projects → sessions → messages, plus full-text search) via
 * `services/hubApi.ts`. Rendered as its own mode — archived history spans
 * machines and outlives local retention, so it is presented separately from
 * the local provider tree, with provenance (machine hostname) visible.
 */

import type { HubConfig } from "../../services/hubApi";

export interface ArchiveBrowserProps {
  /** Hub connection; callers normally derive this from user settings. */
  config: HubConfig;
}

export function ArchiveBrowser(_props: ArchiveBrowserProps) {
  void _props;
  return <div data-testid="archive-browser" />;
}
