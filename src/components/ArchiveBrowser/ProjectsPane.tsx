/**
 * Browse's first pane: identity-grouped projects (one entry per repo
 * identity), with the selected group's members inspectable inline —
 * locations, worktree/linked labels, and the alias link/unlink affordances.
 *
 * `hidden` is the mobile drill-down state, which only the parent knows: below
 * `md` the three Browse panes stack and exactly one level is visible.
 */

import { Link2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "@/lib/utils";
import { getProviderLabel } from "@/utils/providers";
import type { ProjectGroup } from "./projectGrouping";
import type { HubIdentity } from "../../services/hubApi";

export interface ProjectsPaneProps {
  groups: ProjectGroup[];
  /** The selected group's CURRENT incarnation, or null when none is selected. */
  activeGroup: ProjectGroup | null;
  identities: HubIdentity[];
  isLoading: boolean;
  error: string | null;
  aliasError: string | null;
  /** Collapsed by the mobile drill-down (a deeper level is showing). */
  hidden: boolean;
  onSelectGroup: (group: ProjectGroup) => void;
  onLinkAlias: (projectPath: string, identityKey: string) => void;
  onUnlinkAlias: (aliasId: number) => void;
}

export function ProjectsPane({
  groups,
  activeGroup,
  identities,
  isLoading,
  error,
  aliasError,
  hidden,
  onSelectGroup,
  onLinkAlias,
  onUnlinkAlias,
}: ProjectsPaneProps) {
  const { t } = useTranslation();

  return (
    <div
      className={cn(
        "w-full md:w-60 md:shrink-0 overflow-y-auto border border-border/50 rounded-md",
        hidden && "hidden md:block"
      )}
    >
      <p className="px-2 py-1.5 text-px12 font-medium text-muted-foreground">
        {t("settings.archiveHub.browser.projects.title")}
      </p>
      {isLoading && (
        <p className="px-2 py-1 text-px14 text-muted-foreground">
          {t("settings.archiveHub.browser.projects.loading")}
        </p>
      )}
      {error && <p className="px-2 py-1 text-px14 text-destructive">{error}</p>}
      {!isLoading && !error && groups.length === 0 && (
        <p className="px-2 py-1 text-px14 text-muted-foreground">
          {t("settings.archiveHub.browser.projects.empty")}
        </p>
      )}
      <ul>
        {groups.map((group) => {
          const isSelected = activeGroup?.key === group.key;
          const identityInfo = group.identityKey
            ? identities.find((i) => i.identity_key === group.identityKey)
            : undefined;
          // A path already listed under LOCATIONS must not double as
          // a suggestion (guards against hubs predating the
          // fingerprinted-anywhere orphan exclusion).
          const orphanSuggestions =
            identityInfo?.suggestions.filter(
              (s) =>
                s.kind === "orphan_path" &&
                s.project_path &&
                !group.paths.includes(s.project_path)
            ) ?? [];
          return (
            <li key={group.key}>
              <button
                type="button"
                data-testid="project-group"
                onClick={() => onSelectGroup(group)}
                className={`w-full text-left px-2 py-2 text-px14 hover:bg-muted ${
                  isSelected ? "bg-accent/15 dark:bg-accent/25" : ""
                }`}
                title={group.paths.join("\n")}
              >
                <p className="truncate">
                  {group.displayName}
                  {group.disambiguator && (
                    <span className="text-px12 text-muted-foreground">
                      {" — "}
                      {group.disambiguator}
                    </span>
                  )}
                </p>
                <p className="text-px12 text-muted-foreground truncate">
                  {group.machines.join(", ")}
                  {group.providers.map((provider) => (
                    <span
                      key={provider}
                      className="ml-1.5 rounded border border-border bg-muted/50 px-1 py-px text-foreground/75"
                    >
                      {getProviderLabel(t, provider)}
                    </span>
                  ))}
                  {group.worktreePaths.length > 0 && (
                    <span className="ml-1.5 rounded bg-info/10 text-info px-1 py-px">
                      {t("settings.archiveHub.identity.worktree")}
                    </span>
                  )}
                </p>
              </button>
              {/* Member inspection: locations, worktree/linked labels,
                  alias link/unlink affordances. */}
              {isSelected &&
                (group.paths.length > 1 ||
                  orphanSuggestions.length > 0 ||
                  (identityInfo?.aliases.length ?? 0) > 0 ||
                  group.worktreePaths.length > 0) && (
                  <div
                    data-testid="identity-members"
                    className="px-2 pb-2 space-y-2"
                  >
                    {/* Confirmed members: paths already in this
                        identity's scope. Solid accent rail continues the
                        selected row's accent wash. */}
                    <div className="space-y-1">
                      <p className="text-px11 font-medium uppercase tracking-wide text-accent">
                        {t("settings.archiveHub.identity.locations")}
                      </p>
                      <div className="space-y-1 border-l-2 border-accent/40 pl-2">
                        {group.paths.map((path) => {
                          const alias = identityInfo?.aliases.find(
                            (a) => a.project_path === path
                          );
                          return (
                            <div
                              key={path}
                              className="flex items-center gap-1.5 text-px12 text-foreground/70"
                            >
                              <span className="truncate" title={path}>
                                {path}
                              </span>
                              {group.worktreePaths.includes(path) && (
                                <span className="shrink-0 rounded bg-info/10 text-info px-1 py-px">
                                  {t("settings.archiveHub.identity.worktree")}
                                </span>
                              )}
                              {alias && (
                                <>
                                  <span className="shrink-0 rounded border border-accent/30 bg-accent/10 px-1 py-px text-accent">
                                    {t("settings.archiveHub.identity.linked")}
                                  </span>
                                  <button
                                    type="button"
                                    data-testid="identity-unlink"
                                    onClick={() => onUnlinkAlias(alias.id)}
                                    className="shrink-0 rounded border border-border px-1 py-px text-muted-foreground transition-colors hover:border-destructive/40 hover:bg-destructive/10 hover:text-destructive"
                                  >
                                    {t("settings.archiveHub.identity.unlink")}
                                  </button>
                                </>
                              )}
                            </div>
                          );
                        })}
                      </div>
                    </div>

                    {/* Link candidates: paths the hub suspects belong
                        here but cannot prove by git fingerprint. Dashed
                        rail + dimmer text so they never read as members
                        until the user links them. */}
                    {orphanSuggestions.length > 0 && (
                      <div className="space-y-1">
                        <p
                          className="text-px11 font-medium uppercase tracking-wide text-muted-foreground"
                          title={t(
                            "settings.archiveHub.identity.suggestionHint"
                          )}
                        >
                          {t("settings.archiveHub.identity.suggested")}
                        </p>
                        <div className="space-y-1 border-l-2 border-dashed border-border pl-2">
                          {orphanSuggestions.map((suggestion) => (
                            <div
                              key={suggestion.project_path}
                              className="flex items-center gap-1.5 text-px12 text-muted-foreground/80"
                            >
                              <span
                                className="truncate"
                                title={`${suggestion.project_path} — ${t(
                                  "settings.archiveHub.identity.suggestionHint"
                                )}`}
                              >
                                {suggestion.project_path}
                              </span>
                              <button
                                type="button"
                                data-testid="identity-link"
                                title={t(
                                  "settings.archiveHub.identity.linkHint"
                                )}
                                onClick={() =>
                                  onLinkAlias(
                                    suggestion.project_path!,
                                    group.identityKey!
                                  )
                                }
                                className="shrink-0 flex items-center gap-1 rounded border border-accent/30 bg-accent/10 px-1.5 py-px font-medium text-accent transition-colors hover:bg-accent hover:text-accent-foreground"
                              >
                                <Link2 className="w-3 h-3" aria-hidden="true" />
                                {t("settings.archiveHub.identity.link")}
                              </button>
                            </div>
                          ))}
                        </div>
                      </div>
                    )}

                    {aliasError && (
                      <p className="text-px12 text-destructive">{aliasError}</p>
                    )}
                  </div>
                )}
            </li>
          );
        })}
      </ul>
    </div>
  );
}
