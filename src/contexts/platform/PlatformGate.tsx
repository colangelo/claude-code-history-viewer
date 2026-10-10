import type { ReactNode } from "react";
import { usePlatform } from "./context";

export function MobileOnly({ children }: { children: ReactNode }) {
  const { isMobile } = usePlatform();
  return isMobile ? <>{children}</> : null;
}
