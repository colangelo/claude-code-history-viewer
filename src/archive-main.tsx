/**
 * Entry point for the standalone static archive webapp
 * (spec: openspec/specs/static-archive-webapp/spec.md).
 *
 * Boots only what the hub browser needs: i18n, theme/platform/modal
 * providers (the same stack the WebUI runs in a plain browser), and the
 * ConnectGate. Deliberately NO app store bootstrap, NO auth flow, NO
 * `services/api.ts` adapter — the bundle must work served from any static
 * host with the hub API as its only network dependency.
 */

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { OverlayScrollbars } from "overlayscrollbars";
import "overlayscrollbars/overlayscrollbars.css";
import "./index.css";
import "./scrollbar.css";
import "./i18n";
import { ErrorBoundary } from "./components/ErrorBoundary.tsx";
import { PlatformProvider } from "./contexts/platform";
import { ThemeProvider } from "./contexts/theme/ThemeProvider.tsx";
import { ModalProvider } from "./contexts/modal/ModalProvider.tsx";
import { Toaster } from "sonner";
import { ConnectGate } from "./components/ArchiveBrowser/ConnectGate.tsx";

OverlayScrollbars(document.body, {
  scrollbars: {
    theme: "os-theme-custom",
    autoHide: "leave",
    autoHideDelay: 400,
  },
});

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ErrorBoundary>
      <PlatformProvider>
        <ThemeProvider>
          <ModalProvider>
            <div className="h-screen p-3">
              <ConnectGate />
            </div>
            <Toaster />
          </ModalProvider>
        </ThemeProvider>
      </PlatformProvider>
    </ErrorBoundary>
  </StrictMode>
);
