import { createContext, useContext } from "react";

export interface PlatformContextValue {
  isMobile: boolean;
}

export const PlatformContext = createContext<PlatformContextValue | null>(null);

export function usePlatform() {
  const value = useContext(PlatformContext);

  if (!value) {
    throw new Error("usePlatform must be used within a PlatformProvider");
  }

  return value;
}
