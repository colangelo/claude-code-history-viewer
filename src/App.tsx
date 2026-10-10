import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { useAppStore } from "./store/useAppStore";
import { useAnalytics } from "./hooks/useAnalytics";
import { useResizablePanel } from "./hooks/useResizablePanel";
import { useAppKeyboard } from "./hooks/useAppKeyboard";
import { useAppInitialization } from "./hooks/useAppInitialization";
import { useLiveStatusMessage } from "./hooks/useLiveStatusMessage";
import { useExternalLinks } from "./hooks/useExternalLinks";
import { usePlatform } from "@/contexts/platform";
import { AppLayout } from "@/layouts/AppLayout";
import {
  type ClaudeSession,
  type ClaudeProject,
  type SessionTokenStats,
  type GroupingMode,
} from "./types";
import { getProviderLabel, normalizeProviderIds } from "./utils/providers";

import "./App.css";

function App() {
  const {
    projects,
    sessions,
    selectedProject,
    selectedSession,
    messages,
    isLoading,
    isLoadingProjects,
    isLoadingSessions,
    isLoadingMessages,
    isLoadingTokenStats,
    error,
    sessionTokenStats,
    sessionConversationTokenStats,
    projectTokenStats,
    projectConversationTokenStats,
    projectTokenStatsSummary,
    projectConversationTokenStatsSummary,
    projectTokenStatsPagination,
    sessionSearch,
    selectProject,
    selectSession,
    clearProjectSelection,
    setSessionSearchQuery,
    setSearchFilterType,
    goToNextMatch,
    goToPrevMatch,
    clearSessionSearch,
    loadGlobalStats,
    setAnalyticsCurrentView,
    loadMoreProjectTokenStats,
    loadMoreRecentEdits,
    updateUserSettings,
    getGroupedProjects,
    getDirectoryGroupedProjects,
    getEffectiveGroupingMode,
    hideProject,
    unhideProject,
    isProjectHidden,
    dateFilter,
    setDateFilter,
    isNavigatorOpen,
    toggleNavigator,
    activeProviders,
  } = useAppStore();

  const {
    state: analyticsState,
    actions: analyticsActions,
    computed,
  } = useAnalytics();

  const { t } = useTranslation();
  const { isMobile } = usePlatform();

  // Side-effect hooks (no return value)
  useAppKeyboard();
  useExternalLinks();
  useAppInitialization({ isMessagesView: computed.isMessagesView });

  const liveStatusMessage = useLiveStatusMessage({
    isLoading,
    isAnyLoading: computed.isAnyLoading,
    isLoadingMessages,
    isLoadingProjects,
    isLoadingSessions,
  });

  const globalOverviewDescription = useMemo(() => {
    const normalized = normalizeProviderIds(activeProviders);

    if (normalized.length === 0) {
      return t("analytics.globalOverviewDescription");
    }

    const labels = normalized.map((providerId) =>
      getProviderLabel((key, fallback) => t(key, fallback), providerId)
    );

    if (labels.length === 1) {
      return t(
        "analytics.globalOverviewDescriptionSingleProvider",
        "Aggregated statistics for {{provider}} projects on your machine",
        { provider: labels[0] }
      );
    }

    return t(
      "analytics.globalOverviewDescriptionMultiProvider",
      "Aggregated statistics for selected providers ({{providers}}) on your machine",
      { providers: labels.join(", ") }
    );
  }, [activeProviders, t]);

  // Local state
  const [isViewingGlobalStats, setIsViewingGlobalStats] = useState(false);
  const [isSidebarCollapsed, setIsSidebarCollapsed] = useState(false);
  const [isMobileSidebarOpen, setIsMobileSidebarOpen] = useState(false);

  // A selected session and the project-agnostic global-stats overview are
  // mutually exclusive — entering global stats clears the project/session
  // (clearProjectSelection). The sidebar resets this flag explicitly on
  // session select, but the global search modal selects sessions via store
  // actions and can't reach this local state, so it would otherwise stay in
  // the global-stats view and hide the navigated conversation. Guarantee the
  // exit here whenever a session becomes selected (issue #390).
  useEffect(() => {
    if (selectedSession) {
      setIsViewingGlobalStats(false);
    }
  }, [selectedSession]);

  // Sidebar resize
  const {
    width: sidebarWidth,
    isResizing: isSidebarResizing,
    handleMouseDown: handleSidebarResizeStart,
  } = useResizablePanel({
    defaultWidth: 256,
    minWidth: 200,
    maxWidth: 800,
    storageKey: "sidebar-width",
  });

  // Navigator resize (right sidebar)
  const {
    width: navigatorWidth,
    isResizing: isNavigatorResizing,
    handleMouseDown: handleNavigatorResizeStart,
  } = useResizablePanel({
    defaultWidth: 280,
    minWidth: 200,
    maxWidth: 400,
    storageKey: "navigator-width",
    direction: "left",
  });

  const handleGlobalStatsClick = useCallback(() => {
    setIsViewingGlobalStats(true);
    clearProjectSelection();
    setAnalyticsCurrentView("analytics");
    void loadGlobalStats();
  }, [clearProjectSelection, loadGlobalStats, setAnalyticsCurrentView]);

  const handleToggleSidebar = useCallback(() => {
    setIsSidebarCollapsed((prev) => !prev);
  }, []);

  // Project grouping
  const groupingMode = getEffectiveGroupingMode();
  const { groups: worktreeGroups, ungrouped: ungroupedProjects } =
    getGroupedProjects();
  const { groups: directoryGroups } = getDirectoryGroupedProjects();

  const handleGroupingModeChange = useCallback(
    (newMode: GroupingMode) => {
      updateUserSettings({
        groupingMode: newMode,
        worktreeGrouping: newMode === "worktree",
        worktreeGroupingUserSet: true,
      });
    },
    [updateUserSettings]
  );

  const handleSessionSelect = useCallback(
    async (session: ClaudeSession) => {
      try {
        setIsViewingGlobalStats(false);
        setAnalyticsCurrentView("messages");

        // Find the project this session belongs to.
        //
        // Comparing `project.name === session.project_name` is unreliable
        // across providers: each provider picks its own way to turn a raw
        // on-disk directory (e.g. "Users-foo-Projects-bar") into a sidebar
        // display label, and `session.project_name` is set by the loader,
        // not by the sidebar. CodeBuddy in particular shortens
        // `dir_name.rsplit('-').next()` for its sidebar label while the
        // session loader keeps the encoded form — the two never match,
        // so the previous equality check failed to switch to the right
        // project when a session was selected.
        //
        // The path prefix is the one signal that's stable everywhere:
        // a session's `file_path` always lives under its project's `path`.
        // Match on that first, fall back to the name equality only when
        // `file_path` is unavailable.
        //
        // The prefix match must respect the path-segment boundary so
        // sibling projects sharing a parent dir don't collide — without
        // this, `/a/proj` would also match a session under `/a/proj2`.
        const findProjectForSession = (s: ClaudeSession) => {
          if (s.file_path) {
            const fp = s.file_path;
            const byPath = projects.find((p) => {
              if (!fp.startsWith(p.path)) return false;
              if (fp.length === p.path.length) return true;
              const next = fp.charAt(p.path.length);
              return next === "/" || next === "\\";
            });
            if (byPath) return byPath;
          }
          return projects.find((p) => p.name === s.project_name);
        };

        const targetProject = findProjectForSession(session);
        const currentProject = useAppStore.getState().selectedProject;
        if (
          targetProject &&
          (!currentProject || currentProject.path !== targetProject.path)
        ) {
          await selectProject(targetProject);
        }

        await selectSession(session);
      } catch (error) {
        // Per CLAUDE.md "에러 처리": async failures need user-visible
        // feedback (toast/alert). console.error alone leaves the user
        // staring at a sidebar that didn't react to their click.
        console.error("Failed to select session:", error);
        const message = error instanceof Error ? error.message : String(error);
        toast.error(`${t("session.selectError")}: ${message}`);
      }
    },
    [projects, selectProject, selectSession, setAnalyticsCurrentView, t]
  );

  const handleTokenStatClick = useCallback(
    (stats: SessionTokenStats) => {
      const session = sessions.find(
        (s) =>
          s.actual_session_id === stats.session_id ||
          s.session_id === stats.session_id
      );

      if (session) {
        handleSessionSelect(session);
      } else {
        console.warn("Session not found in loaded list:", stats.session_id);
      }
    },
    [sessions, handleSessionSelect]
  );

  const handleProjectSelect = useCallback(
    async (project: ClaudeProject) => {
      const currentProject = useAppStore.getState().selectedProject;

      if (currentProject?.path === project.path) {
        clearProjectSelection();
        return;
      }

      const activeView = useAppStore.getState().analytics.currentView;
      setIsViewingGlobalStats(false);

      analyticsActions.clearAll();
      setDateFilter({ start: null, end: null });

      await selectProject(project);

      try {
        if (activeView === "tokenStats") {
          await analyticsActions.switchToTokenStats();
        } else if (activeView === "board") {
          await analyticsActions.switchToBoard();
        } else if (activeView === "recentEdits") {
          await analyticsActions.switchToRecentEdits();
        } else if (activeView === "analytics") {
          await analyticsActions.switchToAnalytics();
        } else if (activeView === "settings") {
          analyticsActions.switchToSettings();
        } else {
          analyticsActions.switchToMessages();
        }
      } catch (error) {
        console.error(`Failed to auto-load ${activeView} view:`, error);
      }
    },
    [clearProjectSelection, selectProject, analyticsActions, setDateFilter]
  );

  const handleSessionHover = useCallback(
    (session: ClaudeSession) => {
      if (computed.isBoardView) {
        useAppStore.getState().setSelectedSession(session);
      }
    },
    [computed.isBoardView]
  );

  return (
    <AppLayout
      projects={projects}
      sessions={sessions}
      selectedProject={selectedProject}
      selectedSession={selectedSession}
      messages={messages}
      isLoading={isLoading}
      isLoadingProjects={isLoadingProjects}
      isLoadingSessions={isLoadingSessions}
      isLoadingMessages={isLoadingMessages}
      isLoadingTokenStats={isLoadingTokenStats}
      error={error}
      sessionTokenStats={sessionTokenStats}
      sessionConversationTokenStats={sessionConversationTokenStats}
      projectTokenStats={projectTokenStats}
      projectConversationTokenStats={projectConversationTokenStats}
      projectTokenStatsSummary={projectTokenStatsSummary}
      projectConversationTokenStatsSummary={projectConversationTokenStatsSummary}
      projectTokenStatsPagination={projectTokenStatsPagination}
      sessionSearch={sessionSearch}
      dateFilter={dateFilter}
      analyticsState={analyticsState}
      analyticsActions={analyticsActions}
      computed={computed}
      isMobile={isMobile}
      isViewingGlobalStats={isViewingGlobalStats}
      isSidebarCollapsed={isSidebarCollapsed}
      isMobileSidebarOpen={isMobileSidebarOpen}
      setIsMobileSidebarOpen={setIsMobileSidebarOpen}
      setIsViewingGlobalStats={setIsViewingGlobalStats}
      sidebarWidth={sidebarWidth}
      isSidebarResizing={isSidebarResizing}
      handleSidebarResizeStart={handleSidebarResizeStart}
      navigatorWidth={navigatorWidth}
      isNavigatorResizing={isNavigatorResizing}
      handleNavigatorResizeStart={handleNavigatorResizeStart}
      isNavigatorOpen={isNavigatorOpen}
      toggleNavigator={toggleNavigator}
      groupingMode={groupingMode}
      worktreeGroups={worktreeGroups}
      directoryGroups={directoryGroups}
      ungroupedProjects={ungroupedProjects}
      handleProjectSelect={handleProjectSelect}
      handleSessionSelect={handleSessionSelect}
      handleSessionHover={handleSessionHover}
      handleGlobalStatsClick={handleGlobalStatsClick}
      handleToggleSidebar={handleToggleSidebar}
      handleGroupingModeChange={handleGroupingModeChange}
      handleTokenStatClick={handleTokenStatClick}
      hideProject={hideProject}
      unhideProject={unhideProject}
      isProjectHidden={isProjectHidden}
      setDateFilter={setDateFilter}
      setSessionSearchQuery={setSessionSearchQuery}
      setSearchFilterType={setSearchFilterType}
      clearSessionSearch={clearSessionSearch}
      goToNextMatch={goToNextMatch}
      goToPrevMatch={goToPrevMatch}
      loadMoreProjectTokenStats={loadMoreProjectTokenStats}
      loadMoreRecentEdits={loadMoreRecentEdits}
      globalOverviewDescription={globalOverviewDescription}
      liveStatusMessage={liveStatusMessage}
    />
  );
}

export default App;
