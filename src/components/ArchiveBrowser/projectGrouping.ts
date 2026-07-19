/**
 * Identity grouping of hub project rows (spec:
 * openspec/specs/archive-journal-ui — "Identity-grouped project surfaces").
 *
 * Groups `/v1/projects` rows by `identity_key`, falling back to the project
 * path for fingerprint-less rows (path IS identity there — and same-path rows
 * from several machines still fold, matching the journal's path fold).
 * Aliased paths (from `/v1/identities`) fold their rows into the aliased
 * identity so a moved repo's dead path never shows up twice.
 *
 * Display names follow the single humanize-a-path rule of the archive webapp:
 * Windows-tolerant basename, with a dimmed parent-directory suffix only when
 * two visible groups would collide (a fork checked out elsewhere stays
 * distinguishable and is never merged).
 */

import type { HubIdentity, HubProject } from "../../services/hubApi";

export interface ProjectGroup {
  /** Stable list key: the identity key, or `path:<project_path>` fallback. */
  key: string;
  /** Set for fingerprint-grouped projects; null for path-identified ones. */
  identityKey: string | null;
  /** Basename of the most recently active member path. */
  displayName: string;
  /** Parent-directory suffix, present only when display names collide. */
  disambiguator: string | null;
  /** Every row in the group (all machines, providers, paths). */
  rows: HubProject[];
  /** Distinct member paths, most recently active first. */
  paths: string[];
  /** Paths where EVERY row is a linked worktree (the excludable unit). */
  worktreePaths: string[];
  machines: string[];
  providers: string[];
  /** Max `last_modified` across rows (ISO string; null when unknown). */
  lastModified: string | null;
}

/** Windows-tolerant basename — same rule as `JournalEntryCard`. */
export function pathBasename(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

function pathParent(path: string): string | null {
  const parts = path.split(/[\\/]/).filter(Boolean);
  parts.pop();
  return parts.pop() ?? null;
}

function maxIso(a: string | null, b: string | null): string | null {
  if (a == null) return b;
  if (b == null) return a;
  return a >= b ? a : b;
}

/** Map alias path → identity key from a `/v1/identities` response. */
export function aliasKeyByPath(identities: HubIdentity[]): Map<string, string> {
  const map = new Map<string, string>();
  for (const identity of identities) {
    for (const alias of identity.aliases) {
      map.set(alias.project_path, identity.identity_key);
    }
  }
  return map;
}

export interface GroupProjectsOptions {
  /** Alias path → identity key (folds dead paths into their identity). */
  aliases?: Map<string, string>;
  /** When false, worktree-only paths are dropped from groups. */
  showWorktrees?: boolean;
}

export function groupProjects(
  projects: HubProject[],
  options?: GroupProjectsOptions
): ProjectGroup[] {
  const aliases = options?.aliases;
  const showWorktrees = options?.showWorktrees ?? true;

  // Pass 1 — which identity claims each path (fingerprinted or aliased rows).
  // A NULL-identity row whose path an identity already owns must fold INTO
  // that identity, not spawn a twin `path:` group: rows ingested by a
  // pre-fingerprint daemon (or a provider pass that hasn't re-reported yet)
  // otherwise duplicate every cross-provider repo in the sidebar/filter.
  // A path claimed by TWO identities is contested → left as its own group.
  const identityOwningPath = new Map<string, string | null>();
  for (const row of projects) {
    const identityKey =
      row.identity_key ?? aliases?.get(row.project_path) ?? null;
    if (!identityKey) continue;
    const prev = identityOwningPath.get(row.project_path);
    if (prev === undefined) {
      identityOwningPath.set(row.project_path, identityKey);
    } else if (prev !== identityKey) {
      identityOwningPath.set(row.project_path, null); // contested
    }
  }

  const groups = new Map<string, ProjectGroup>();
  for (const row of projects) {
    const identityKey =
      row.identity_key ??
      aliases?.get(row.project_path) ??
      identityOwningPath.get(row.project_path) ??
      null;
    const key = identityKey ?? `path:${row.project_path}`;
    let group = groups.get(key);
    if (!group) {
      group = {
        key,
        identityKey,
        displayName: "",
        disambiguator: null,
        rows: [],
        paths: [],
        worktreePaths: [],
        machines: [],
        providers: [],
        lastModified: null,
      };
      groups.set(key, group);
    }
    group.rows.push(row);
  }

  const result: ProjectGroup[] = [];
  for (const group of groups.values()) {
    // Per-path fold: a path is worktree-only when EVERY row says worktree
    // (a path that is a main checkout anywhere stays a regular member).
    const byPath = new Map<
      string,
      { allWorktree: boolean; lastModified: string | null }
    >();
    for (const row of group.rows) {
      const info = byPath.get(row.project_path);
      if (!info) {
        byPath.set(row.project_path, {
          allWorktree: row.git_worktree,
          lastModified: row.last_modified,
        });
      } else {
        info.allWorktree = info.allWorktree && row.git_worktree;
        info.lastModified = maxIso(info.lastModified, row.last_modified);
      }
    }
    group.worktreePaths = Array.from(byPath.entries())
      .filter(([, info]) => info.allWorktree)
      .map(([path]) => path);

    if (!showWorktrees) {
      const hidden = new Set(group.worktreePaths);
      group.rows = group.rows.filter((r) => !hidden.has(r.project_path));
      for (const path of hidden) byPath.delete(path);
      if (group.rows.length === 0) continue; // worktree-only group hidden
    }

    group.paths = Array.from(byPath.entries())
      .sort(([, a], [, b]) => (a.lastModified ?? "") < (b.lastModified ?? "") ? 1 : -1)
      .map(([path]) => path);
    group.machines = Array.from(
      new Set(group.rows.map((r) => r.machine_hostname))
    ).sort();
    group.providers = Array.from(
      new Set(group.rows.map((r) => r.provider))
    ).sort();
    group.lastModified = group.rows.reduce<string | null>(
      (acc, r) => maxIso(acc, r.last_modified),
      null
    );
    group.displayName = pathBasename(group.paths[0] ?? "");
    result.push(group);
  }

  // Dimmed parent-dir suffix only where basenames collide among VISIBLE groups.
  const nameCounts = new Map<string, number>();
  for (const group of result) {
    nameCounts.set(group.displayName, (nameCounts.get(group.displayName) ?? 0) + 1);
  }
  for (const group of result) {
    if ((nameCounts.get(group.displayName) ?? 0) > 1) {
      group.disambiguator = pathParent(group.paths[0] ?? "");
    }
  }

  // Escalate: one parent segment can still tie (same basename AND parent
  // under different grandparents — `_sync/dev/x` vs `other/dev/x`). Deepen
  // the suffix for colliding labels until they differ or the path runs out;
  // two visibly identical options must never render.
  const labelOf = (g: ProjectGroup) =>
    g.disambiguator ? `${g.displayName} — ${g.disambiguator}` : g.displayName;
  for (let depth = 2; depth <= 6; depth++) {
    const byLabel = new Map<string, ProjectGroup[]>();
    for (const g of result) {
      const label = labelOf(g);
      const bucket = byLabel.get(label);
      if (bucket) bucket.push(g);
      else byLabel.set(label, [g]);
    }
    const colliding = Array.from(byLabel.values()).filter(
      (gs) => gs.length > 1
    );
    if (colliding.length === 0) break;
    for (const gs of colliding) {
      for (const g of gs) {
        const parts = (g.paths[0] ?? "").split(/[\\/]/).filter(Boolean);
        parts.pop(); // basename
        const suffix = parts.slice(-depth).join("/");
        if (suffix) g.disambiguator = suffix;
      }
    }
  }

  // Most recently active first — matches the hub's projects ordering.
  result.sort((a, b) => ((a.lastModified ?? "") < (b.lastModified ?? "") ? 1 : -1));
  return result;
}
