import { useMemo, type ReactNode } from "react";
import { useIsMobile } from "@/hooks/useIsMobile";
import { PlatformContext, type PlatformContextValue } from "./context";

export function PlatformProvider({ children }: { children: ReactNode }) {
  const isMobile = useIsMobile();

  const value = useMemo<PlatformContextValue>(() => ({ isMobile }), [isMobile]);

  return (
    <PlatformContext.Provider value={value}>
      {children}
    </PlatformContext.Provider>
  );
}
