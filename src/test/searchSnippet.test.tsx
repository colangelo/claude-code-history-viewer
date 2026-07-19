import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { renderSnippet } from "../utils/searchSnippet";

function renderNodes(snippet: string) {
  return render(<p>{renderSnippet(snippet)}</p>);
}

describe("renderSnippet", () => {
  it("passes through text without markers", () => {
    const { container } = renderNodes("plain text, no markers");
    expect(container.textContent).toBe("plain text, no markers");
    expect(container.querySelectorAll("mark")).toHaveLength(0);
  });

  it("turns a marker pair into a mark node without literal tags", () => {
    const { container } = renderNodes("foo <b>bar</b> baz");
    expect(container.textContent).toBe("foo bar baz");
    const marks = container.querySelectorAll("mark");
    expect(marks).toHaveLength(1);
    expect(marks[0]?.textContent).toBe("bar");
  });

  it("highlights every occurrence", () => {
    const { container } = renderNodes("<b>a</b> mid <b>b</b> end");
    const marks = [...container.querySelectorAll("mark")].map(
      (m) => m.textContent
    );
    expect(marks).toEqual(["a", "b"]);
    expect(container.textContent).toBe("a mid b end");
  });

  it("degrades unpaired markers to literal text", () => {
    const { container } = renderNodes("broken <b>unclosed marker");
    expect(container.textContent).toBe("broken <b>unclosed marker");
    expect(container.querySelectorAll("mark")).toHaveLength(0);
  });

  it("never injects other HTML from message content", () => {
    const { container } = renderNodes('x <b>hit</b> <script>alert("y")</script>');
    expect(container.querySelector("script")).toBeNull();
    expect(container.textContent).toContain('<script>alert("y")</script>');
  });
});
