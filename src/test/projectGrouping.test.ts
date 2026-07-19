import { describe, expect, it } from "vitest";
import {
  aliasKeyByPath,
  groupProjects,
  pathBasename,
} from "../components/ArchiveBrowser/projectGrouping";
import type { HubIdentity, HubProject } from "../services/hubApi";

const KEY = "g:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa|github.com/acme/foo";
const FORK_KEY =
  "g:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa|github.com/upstream/foo";

let nextId = 1;
function row(overrides: Partial<HubProject>): HubProject {
  return {
    id: nextId++,
    provider: "claude",
    project_path: "/home/ac/dev/foo",
    name: null,
    storage_type: null,
    session_count: 1,
    message_count: 1,
    last_modified: "2026-07-10T12:00:00Z",
    machine_id: "m-1",
    machine_hostname: "m4m",
    identity_key: null,
    git_worktree: false,
    git_main_path: null,
    ...overrides,
  };
}

describe("groupProjects", () => {
  it("folds same identity across paths, machines, and providers", () => {
    const groups = groupProjects([
      row({ identity_key: KEY, project_path: "/old/foo", last_modified: "2026-07-01T00:00:00Z" }),
      row({ identity_key: KEY, project_path: "/new/foo", machine_hostname: "mbm5", provider: "codex" }),
    ]);
    expect(groups).toHaveLength(1);
    const g = groups[0]!;
    expect(g.identityKey).toBe(KEY);
    // Most recent member's basename wins; paths newest-first.
    expect(g.displayName).toBe("foo");
    expect(g.paths).toEqual(["/new/foo", "/old/foo"]);
    expect(g.machines).toEqual(["m4m", "mbm5"]);
    expect(g.providers).toEqual(["claude", "codex"]);
  });

  it("keeps a fork (different key) separate, with a disambiguating suffix", () => {
    const groups = groupProjects([
      row({ identity_key: KEY, project_path: "/home/ac/dev/foo" }),
      row({ identity_key: FORK_KEY, project_path: "/home/ac/forks/foo" }),
    ]);
    expect(groups).toHaveLength(2);
    expect(groups.map((g) => g.displayName)).toEqual(["foo", "foo"]);
    const suffixes = groups.map((g) => g.disambiguator).sort();
    expect(suffixes).toEqual(["dev", "forks"]);
  });

  it("falls back to path identity (cross-machine fold) without a fingerprint", () => {
    const groups = groupProjects([
      row({ project_path: "/home/ac/dev/bar", machine_hostname: "m4m" }),
      row({ project_path: "/home/ac/dev/bar", machine_hostname: "mbm5" }),
      row({ project_path: "/home/ac/dev/baz" }),
    ]);
    expect(groups).toHaveLength(2);
    const bar = groups.find((g) => g.displayName === "bar")!;
    expect(bar.identityKey).toBeNull();
    expect(bar.machines).toEqual(["m4m", "mbm5"]);
  });

  it("folds aliased dead paths into their identity's group", () => {
    const identities: HubIdentity[] = [
      {
        identity_key: KEY,
        display_name: "foo",
        members: [],
        aliases: [
          {
            id: 1,
            project_path: "/dead/foo",
            created_by: "machine:x",
            created_at: "2026-07-10T00:00:00Z",
          },
        ],
        suggestions: [],
      },
    ];
    const groups = groupProjects(
      [
        row({ identity_key: KEY, project_path: "/new/foo" }),
        row({ project_path: "/dead/foo", last_modified: "2026-07-01T00:00:00Z" }),
      ],
      { aliases: aliasKeyByPath(identities) }
    );
    expect(groups).toHaveLength(1);
    expect(groups[0]!.paths).toEqual(["/new/foo", "/dead/foo"]);
  });

  it("labels worktree-only paths and hides them when showWorktrees=false", () => {
    const rows = [
      row({ identity_key: KEY, project_path: "/dev/foo" }),
      row({
        identity_key: KEY,
        project_path: "/dev/foo.feature-x",
        git_worktree: true,
        git_main_path: "/dev/foo",
      }),
    ];
    const shown = groupProjects(rows);
    expect(shown[0]!.worktreePaths).toEqual(["/dev/foo.feature-x"]);
    expect(shown[0]!.paths).toContain("/dev/foo.feature-x");

    const hidden = groupProjects(rows, { showWorktrees: false });
    expect(hidden[0]!.paths).toEqual(["/dev/foo"]);
  });

  it("keeps a path that is a main checkout anywhere (inclusion-safe default)", () => {
    const groups = groupProjects([
      row({ identity_key: KEY, project_path: "/dev/foo" }),
      // Same path: worktree on one machine, main on another.
      row({ identity_key: KEY, project_path: "/dev/shared", git_worktree: true }),
      row({
        identity_key: KEY,
        project_path: "/dev/shared",
        machine_hostname: "mbm5",
        git_worktree: false,
      }),
    ]);
    expect(groups[0]!.worktreePaths).toEqual([]);
  });

  it("drops a worktree-only group entirely when hidden", () => {
    const groups = groupProjects(
      [
        row({
          identity_key: KEY,
          project_path: "/dev/foo.feature-x",
          git_worktree: true,
        }),
      ],
      { showWorktrees: false }
    );
    expect(groups).toHaveLength(0);
  });

  it("sorts groups by most recent activity", () => {
    const groups = groupProjects([
      row({ project_path: "/dev/older", last_modified: "2026-07-01T00:00:00Z" }),
      row({ project_path: "/dev/newer", last_modified: "2026-07-15T00:00:00Z" }),
    ]);
    expect(groups.map((g) => g.displayName)).toEqual(["newer", "older"]);
  });
});

describe("pathBasename", () => {
  it("handles POSIX and Windows separators", () => {
    expect(pathBasename("/home/ac/dev/foo")).toBe("foo");
    expect(pathBasename("C:\\Users\\ac\\dev\\foo")).toBe("foo");
    expect(pathBasename("/trailing/slash/")).toBe("slash");
  });
});
