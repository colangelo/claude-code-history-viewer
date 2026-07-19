/**
 * Renders a hub search snippet, turning the FTS highlight markers (`<b>…</b>`
 * emitted by the hub's snippet function) into real `<mark>` nodes. The snippet
 * is message text, so it is NEVER injected as HTML — we split on the marker
 * pair and emit React nodes. Unpaired/nested markers degrade to literal text.
 */

import type { ReactNode } from "react";

const MARKER = /<b>(.*?)<\/b>/g;

export function renderSnippet(snippet: string): ReactNode[] {
  const nodes: ReactNode[] = [];
  let last = 0;
  let key = 0;
  for (const match of snippet.matchAll(MARKER)) {
    const index = match.index ?? 0;
    if (index > last) nodes.push(snippet.slice(last, index));
    nodes.push(
      <mark
        key={key++}
        className="bg-accent/20 text-inherit font-medium rounded-sm px-0.5"
      >
        {match[1]}
      </mark>
    );
    last = index + match[0].length;
  }
  if (last < snippet.length) nodes.push(snippet.slice(last));
  return nodes;
}
