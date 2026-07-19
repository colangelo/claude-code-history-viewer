/**
 * Marks a renderer subtree as displaying ARCHIVED history (hub rows rendered
 * independently, so a tool_use never sees its paired tool_result). Live-state
 * affordances that are meaningless there — the "Pending" status on tool cards
 * whose results simply live in the next row — check this to stay hidden.
 * Defaults to false so the desktop/WebUI live viewer is unaffected.
 */

import { createContext, useContext } from "react";

export const ArchiveRenderContext = createContext(false);

export function useIsArchiveRender(): boolean {
  return useContext(ArchiveRenderContext);
}
